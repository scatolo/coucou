// The Feishin pill — media controls and state for Feishin.
//
// Linux: Feishin publishes standard MPRIS v2 on the session bus at
// `org.mpris.MediaPlayer2.Feishin`. We block on the bus and wake on
// PropertiesChanged, Seeked, and NameOwnerChanged signals.
//
// Windows: Feishin does not use D-Bus. State is driven through Feishin's
// native Remote WebSocket server (ws://127.0.0.1:4333) in the frontend.
// Commands here handle process launching and install checks.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::Serialize;
use tauri::AppHandle;

pub const PILL_ID: &str = "integration_feishin";
/// Where "Get Feishin" leads when it isn't installed.
pub const DOWNLOAD_URL: &str = "https://github.com/jeffvli/feishin/releases";
/// Island events: the player's state, and a track's cover as a data URL.
const STATE_EVENT: &str = "feishin";
const ARTWORK_EVENT: &str = "feishin-artwork";
/// The biggest cover we read (3 MB limit).
const ARTWORK_LIMIT: usize = 3 * 1024 * 1024;

// ── Values ────────────────────────────────────────────────────────────────────

/// A D-Bus value as far as MPRIS needs it, so the parsing is plain Rust that
/// can be tested without a bus.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Strs(Vec<String>),
    Int(i64),
    Float(f64),
    Bool(bool),
    Map(Vec<(String, Value)>),
    Other,
}

impl Value {
    fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    fn int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    /// Text, or a list of texts joined (xesam:artist is a list).
    fn text(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Strs(list) => list
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", "),
            _ => String::new(),
        }
    }
}

fn find<'a>(map: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

// ── State ─────────────────────────────────────────────────────────────────────

/// The track Feishin has loaded.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Seconds.
    pub duration: f64,
    pub art_url: Option<String>,
    /// mpris:trackid as Feishin gave it, for SetPosition.
    #[serde(skip)]
    pub object_path: String,
}

/// What the island is told, whenever it changes.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    /// Feishin owns its name on the bus.
    pub running: bool,
    /// Feishin is installed and can be launched.
    pub installed: bool,
    pub track: Option<Track>,
    pub playing: bool,
    /// Seconds at `position_at`; while playing, real position runs on from there.
    pub position: f64,
    /// Unix milliseconds.
    pub position_at: f64,
    pub shuffle: bool,
    /// LoopStatus is not "None".
    pub repeat: bool,
    /// 0…100.
    pub volume: i32,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            running: false,
            installed: false,
            track: None,
            playing: false,
            position: 0.0,
            position_at: 0.0,
            shuffle: false,
            repeat: false,
            volume: 50,
        }
    }
}

/// What a change touched, so the caller knows what to read next.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Applied {
    pub track_changed: bool,
    pub status_changed: bool,
    pub position_set: bool,
}

impl PlayerState {
    /// Where playback is at `now_ms`, extrapolated from the last anchor and
    /// capped at the track's end.
    pub fn position_at_time(&self, now_ms: f64) -> f64 {
        let elapsed = if self.playing { ((now_ms - self.position_at) / 1000.0).max(0.0) } else { 0.0 };
        let p = self.position + elapsed;
        match &self.track {
            Some(t) if t.duration > 0.0 => p.min(t.duration),
            _ => p,
        }
    }

    fn anchor(&mut self, position: f64, now_ms: f64) {
        self.position = position.max(0.0);
        self.position_at = now_ms;
    }

    /// Feishin is gone, or the pill is off: nothing is playing.
    pub fn clear(&mut self, now_ms: f64) {
        self.track = None;
        self.playing = false;
        self.anchor(0.0, now_ms);
    }

    /// Applies Player properties (GetAll, or a PropertiesChanged).
    pub fn apply(&mut self, props: &[(String, Value)], now_ms: f64) -> Applied {
        let mut applied = Applied::default();
        let rank = |k: &str| match k {
            "Metadata" => 0,
            "PlaybackStatus" => 1,
            "Position" => 3,
            _ => 2,
        };
        let mut ordered: Vec<&(String, Value)> = props.iter().collect();
        ordered.sort_by_key(|(k, _)| rank(k));
        let mut stopped = false;
        for (key, value) in ordered {
            match key.as_str() {
                "Metadata" => {
                    let track = match value {
                        Value::Map(map) => parse_metadata(map),
                        _ => None,
                    };
                    let same = match (&self.track, &track) {
                        (Some(a), Some(b)) => a.id == b.id && a.object_path == b.object_path,
                        (None, None) => true,
                        _ => false,
                    };
                    if !same {
                        applied.track_changed = true;
                        self.anchor(0.0, now_ms);
                    }
                    self.track = track;
                }
                "PlaybackStatus" => {
                    let Some(status) = value.str() else { continue };
                    stopped = status == "Stopped";
                    let playing = status == "Playing";
                    if playing != self.playing {
                        let at = self.position_at_time(now_ms);
                        self.anchor(at, now_ms);
                        self.playing = playing;
                        applied.status_changed = true;
                    }
                }
                "Shuffle" => {
                    if let Value::Bool(on) = value {
                        self.shuffle = *on;
                    }
                }
                "LoopStatus" => {
                    if let Some(s) = value.str() {
                        self.repeat = s != "None";
                    }
                }
                "Volume" => {
                    if let Value::Float(v) = value {
                        self.volume = (v * 100.0).round().clamp(0.0, 100.0) as i32;
                    }
                }
                "Position" => {
                    if let Some(us) = value.int() {
                        self.anchor(us as f64 / 1_000_000.0, now_ms);
                        applied.position_set = true;
                    }
                }
                _ => {}
            }
        }
        if stopped && self.track.is_some() {
            self.clear(now_ms);
            applied.track_changed = true;
        }
        if self.track.is_none() {
            self.playing = false;
        }
        applied
    }
}

/// xesam / mpris metadata → Track, or None when nothing is loaded.
pub fn parse_metadata(map: &[(String, Value)]) -> Option<Track> {
    let object_path = find(map, "mpris:trackid").map(Value::text).unwrap_or_default();
    let title = find(map, "xesam:title").map(Value::text).unwrap_or_default();
    if object_path.is_empty() && title.is_empty() {
        return None;
    }
    if object_path.ends_with("/NoTrack") {
        return None;
    }
    // Feishin currently sends (duration * 1e3), whereas standard MPRIS uses 1e6.
    // Handles microseconds (> 10M), milliseconds (> 1K), and seconds safely.
    let duration = find(map, "mpris:length")
        .and_then(Value::int)
        .map(|val| {
            let val = val.max(0);
            if val > 10_000_000 {
                val as f64 / 1_000_000.0
            } else if val > 1_000 {
                val as f64 / 1_000.0
            } else {
                val as f64
            }
        })
        .unwrap_or(0.0);
    let art_url = find(map, "mpris:artUrl").map(Value::text).filter(|s| !s.is_empty());
    let id = feishin_id(&object_path);
    Some(Track {
        id,
        title,
        artist: find(map, "xesam:artist").map(Value::text).unwrap_or_default(),
        album: find(map, "xesam:album").map(Value::text).unwrap_or_default(),
        duration,
        art_url,
        object_path,
    })
}

pub fn feishin_id(trackid: &str) -> String {
    if let Some(rest) = trackid.strip_prefix("/org/mpris/MediaPlayer2/track/") {
        return format!("feishin:track:{rest}");
    }
    if !trackid.is_empty() {
        return trackid.to_string();
    }
    "feishin:track:unknown".to_string()
}

// ── Artwork ───────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub enum ArtSource {
    Http(String),
    File(std::path::PathBuf),
}

pub fn artwork_source(raw: &str) -> Option<ArtSource> {
    let url = reqwest::Url::parse(raw).ok()?;
    match url.scheme() {
        "https" | "http" => Some(ArtSource::Http(url.to_string())),
        "file" => {
            if url.host_str().is_some_and(|h| !h.is_empty() && h != "localhost") {
                return None;
            }
            let path = url.to_file_path().ok()?;
            path.is_absolute().then_some(ArtSource::File(path))
        }
        _ => None,
    }
}

pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

fn data_url(bytes: &[u8]) -> Option<String> {
    let mime = sniff_image(bytes)?;
    Some(format!("data:{mime};base64,{}", crate::claude::base64_for(bytes)))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Artwork {
    art_url: String,
    data_url: String,
}

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn emit_state(app: &AppHandle, state: &PlayerState) {
    use tauri::Emitter;
    let _ = app.emit_to(crate::island::WINDOW_LABEL, STATE_EVENT, state);
}

// ── Commands ──────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn feishin_refresh(app: AppHandle) -> Option<PlayerState> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(move || linux::refresh(&linux::Out::App(app))).await.ok().flatten()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        None
    }
}

#[tauri::command]
pub async fn feishin_control(app: AppHandle, action: String, value: Option<f64>) -> bool {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(move || linux::control(&linux::Out::App(app), &action, value)).await.unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, action, value);
        false
    }
}

#[tauri::command]
pub async fn feishin_open() -> bool {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(linux::open).await.unwrap_or(false)
    }
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(windows::open).await.unwrap_or(false)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        false
    }
}

#[tauri::command]
pub fn feishin_installed() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::installed()
    }
    #[cfg(windows)]
    {
        windows::installed()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        false
    }
}

pub fn sync(app: &AppHandle, active_integrations: &[String]) {
    #[cfg(target_os = "linux")]
    linux::sync(app, active_integrations.iter().any(|id| id == PILL_ID));
    #[cfg(not(target_os = "linux"))]
    let _ = (app, active_integrations);
}

// ── Linux: MPRIS over D-Bus ───────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::{LazyLock, Mutex};
    use std::time::Duration;

    use dbus::arg::{ArgType, PropMap, RefArg};
    use dbus::blocking::stdintf::org_freedesktop_dbus::Properties;
    use dbus::blocking::Connection;
    use dbus::message::MessageType;
    use dbus::Message;
    use tauri::Emitter;

    const BUS: &str = "org.mpris.MediaPlayer2.Feishin";
    const PATH: &str = "/org/mpris/MediaPlayer2";
    const ROOT: &str = "org.mpris.MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
    const DBUS: &str = "org.freedesktop.DBus";
    const CALL: Duration = Duration::from_millis(1500);
    const IDLE_WAIT: Duration = Duration::from_secs(3600);
    const FLATPAK_ID: &str = "org.jeffvli.feishin";

    const RULES: [&str; 3] = [
        "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.mpris.MediaPlayer2.Feishin'",
        "type='signal',sender='org.mpris.MediaPlayer2.Feishin',path='/org/mpris/MediaPlayer2',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
        "type='signal',sender='org.mpris.MediaPlayer2.Feishin',path='/org/mpris/MediaPlayer2',interface='org.mpris.MediaPlayer2.Player',member='Seeked'",
    ];

    struct Shared {
        generation: u64,
        active: bool,
        listener: Option<String>,
        owner: Option<String>,
        state: PlayerState,
    }

    static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| {
        Mutex::new(Shared { generation: 0, active: false, listener: None, owner: None, state: PlayerState::default() })
    });

    static CONTROL: LazyLock<Mutex<Option<Connection>>> = LazyLock::new(|| Mutex::new(None));

    struct ArtCache {
        entries: VecDeque<(String, String)>,
        loading: Option<String>,
    }

    static ART: LazyLock<Mutex<ArtCache>> =
        LazyLock::new(|| Mutex::new(ArtCache { entries: VecDeque::new(), loading: None }));

    #[derive(Clone)]
    pub enum Out {
        App(AppHandle),
        #[cfg(test)]
        Test(std::sync::mpsc::Sender<PlayerState>),
    }

    impl Out {
        fn state(&self, state: &PlayerState) {
            match self {
                Out::App(app) => {
                    emit_state(app, state);
                    want_artwork(app, state);
                }
                #[cfg(test)]
                Out::Test(tx) => {
                    let _ = tx.send(state.clone());
                }
            }
        }
    }

    pub fn value_of(arg: &dyn RefArg) -> Value {
        match arg.arg_type() {
            ArgType::Variant => arg.as_iter().and_then(|mut it| it.next().map(value_of)).unwrap_or(Value::Other),
            ArgType::String | ArgType::ObjectPath | ArgType::Signature => {
                arg.as_str().map(|s| Value::Str(s.to_string())).unwrap_or(Value::Other)
            }
            ArgType::Boolean => arg.as_i64().map(|n| Value::Bool(n != 0)).unwrap_or(Value::Other),
            ArgType::Double => arg.as_f64().map(Value::Float).unwrap_or(Value::Other),
            ArgType::UInt64 => arg.as_u64().map(|n| Value::Int(n.min(i64::MAX as u64) as i64)).unwrap_or(Value::Other),
            ArgType::Byte | ArgType::Int16 | ArgType::UInt16 | ArgType::Int32 | ArgType::UInt32 | ArgType::Int64 => {
                arg.as_i64().map(Value::Int).unwrap_or(Value::Other)
            }
            ArgType::Array => {
                let Some(items) = arg.as_iter() else { return Value::Other };
                if arg.signature().starts_with("a{") {
                    let mut map = Vec::new();
                    let mut items = items;
                    while let (Some(k), Some(v)) = (items.next(), items.next()) {
                        if let Some(key) = k.as_str() {
                            map.push((key.to_string(), value_of(v)));
                        }
                    }
                    Value::Map(map)
                } else {
                    Value::Strs(
                        items
                            .filter_map(|v| match value_of(v) {
                                Value::Str(s) => Some(s),
                                _ => None,
                            })
                            .collect(),
                    )
                }
            }
            _ => Value::Other,
        }
    }

    pub fn props_of(map: &PropMap) -> Vec<(String, Value)> {
        map.iter().map(|(k, v)| (k.clone(), value_of(&v.0))).collect()
    }

    pub fn sync(app: &AppHandle, on: bool) {
        set_active(Out::App(app.clone()), on);
    }

    pub fn set_active(out: Out, on: bool) {
        let wake = {
            let mut s = SHARED.lock().unwrap();
            if s.active == on {
                return;
            }
            s.active = on;
            s.generation += 1;
            s.owner = None;
            s.state = PlayerState { installed: installed(), ..PlayerState::default() };
            s.listener.take()
        };
        if on {
            let generation = SHARED.lock().unwrap().generation;
            let spawned = std::thread::Builder::new()
                .name("coucou-feishin".into())
                .spawn(move || listen(out, generation));
            if let Err(err) = spawned {
                crate::log::line(format!("feishin: no listener thread: {err}"));
            }
        } else {
            if let Some(name) = wake {
                wake_listener(&name);
            }
            let state = SHARED.lock().unwrap().state.clone();
            out.state(&state);
        }
    }

    fn current(generation: u64) -> bool {
        let s = SHARED.lock().unwrap();
        s.active && s.generation == generation
    }

    /// Any message to its unique name gets the listener off the bus.
    fn wake_listener(name: &str) {
        let Ok(msg) = Message::new_method_call(name, "/", "org.freedesktop.DBus.Peer", "Ping") else { return };
        let mut msg = msg;
        msg.set_no_reply(true);
        with_control(|c| {
            let _ = c.channel().send(msg);
            c.channel().flush();
            Ok(())
        });
    }

    fn listen(out: Out, generation: u64) {
        let conn = match Connection::new_session() {
            Ok(c) => c,
            Err(err) => {
                crate::log::line(format!("feishin: no session bus: {err}"));
                return;
            }
        };
        {
            let mut s = SHARED.lock().unwrap();
            if !(s.active && s.generation == generation) {
                return;
            }
            s.listener = Some(conn.unique_name().to_string());
        }
        for rule in RULES {
            if let Err(err) = conn.add_match_no_cb(rule) {
                crate::log::line(format!("feishin: match refused: {err}"));
            }
        }
        read_everything(&conn, &out, generation);

        while current(generation) {
            match conn.channel().blocking_pop_message(IDLE_WAIT) {
                Ok(Some(msg)) => handle(&conn, &out, generation, &msg),
                Ok(None) => {}
                Err(err) => {
                    crate::log::line(format!("feishin: bus lost: {err}"));
                    break;
                }
            }
        }
    }

    fn handle(conn: &Connection, out: &Out, generation: u64, msg: &Message) {
        if msg.msg_type() != MessageType::Signal || !current(generation) {
            return;
        }
        let interface = msg.interface().map(|i| i.to_string()).unwrap_or_default();
        let member = msg.member().map(|m| m.to_string()).unwrap_or_default();
        let sender = msg.sender().map(|s| s.to_string());

        match (interface.as_str(), member.as_str()) {
            (DBUS, "NameOwnerChanged") => {
                let Ok((name, _old, new)) = msg.read3::<String, String, String>() else { return };
                if name != BUS {
                    return;
                }
                if new.is_empty() {
                    let state = {
                        let mut s = SHARED.lock().unwrap();
                        s.owner = None;
                        s.state.running = false;
                        s.state.clear(now_ms());
                        s.state.clone()
                    };
                    out.state(&state);
                } else {
                    read_everything(conn, out, generation);
                }
            }
            (PROPERTIES, "PropertiesChanged") => {
                if !from_owner(sender.as_deref()) {
                    return;
                }
                let Ok((iface, changed, invalidated)) = msg.read3::<String, PropMap, Vec<String>>() else { return };
                if iface != PLAYER {
                    return;
                }
                let mut props = props_of(&changed);
                for name in invalidated {
                    if let Some(v) = get_property(conn, &name) {
                        props.push((name, v));
                    }
                }
                apply_and_emit(conn, out, &props);
            }
            (PLAYER, "Seeked") => {
                if !from_owner(sender.as_deref()) {
                    return;
                }
                let Ok(us) = msg.read1::<i64>() else { return };
                apply_and_emit(conn, out, &[("Position".into(), Value::Int(us))]);
            }
            _ => {}
        }
    }

    fn from_owner(sender: Option<&str>) -> bool {
        let s = SHARED.lock().unwrap();
        match (&s.owner, sender) {
            (Some(owner), Some(sender)) => owner == sender,
            (None, _) => true,
            _ => false,
        }
    }

    fn apply_and_emit(conn: &Connection, out: &Out, props: &[(String, Value)]) {
        let applied = SHARED.lock().unwrap().state.apply(props, now_ms());
        if (applied.status_changed || applied.track_changed) && !applied.position_set {
            if let Some(position) = get_property(conn, "Position") {
                SHARED.lock().unwrap().state.apply(&[("Position".into(), position)], now_ms());
            }
        }
        let state = SHARED.lock().unwrap().state.clone();
        out.state(&state);
    }

    fn get_property(conn: &Connection, name: &str) -> Option<Value> {
        let proxy = conn.with_proxy(BUS, PATH, CALL);
        let v: Box<dyn RefArg> = proxy.get(PLAYER, name).ok()?;
        Some(value_of(&v))
    }

    fn name_owner(conn: &Connection) -> Option<String> {
        let proxy = conn.with_proxy(DBUS, "/org/freedesktop/DBus", CALL);
        let (owner,): (String,) = proxy.method_call(DBUS, "GetNameOwner", (BUS,)).ok()?;
        Some(owner)
    }

    fn read_everything(conn: &Connection, out: &Out, generation: u64) {
        let owner = name_owner(conn);
        let props = owner.as_ref().and_then(|_| {
            let proxy = conn.with_proxy(BUS, PATH, CALL);
            proxy.get_all(PLAYER).ok().map(|m| props_of(&m))
        });
        let state = {
            let mut s = SHARED.lock().unwrap();
            if !(s.active && s.generation == generation) {
                return;
            }
            s.owner = owner;
            s.state.running = s.owner.is_some();
            s.state.clear(now_ms());
            if let Some(props) = props {
                s.state.apply(&props, now_ms());
            }
            s.state.clone()
        };
        out.state(&state);
        if state.running {
            if let Some(position) = get_property(conn, "Position") {
                let state = {
                    let mut s = SHARED.lock().unwrap();
                    s.state.apply(&[("Position".into(), position)], now_ms());
                    s.state.clone()
                };
                out.state(&state);
            }
        }
    }

    fn with_control<F, R>(f: F) -> Option<R>
    where
        F: FnOnce(&Connection) -> Result<R, dbus::Error>,
    {
        let mut guard = CONTROL.lock().unwrap();
        if guard.is_none() {
            *guard = Connection::new_session().ok();
        }
        let bus = guard.as_ref()?;
        match f(bus) {
            Ok(val) => Some(val),
            Err(err) => {
                if err.name() == Some("org.freedesktop.DBus.Error.NoReply")
                    || err.name() == Some("org.freedesktop.DBus.Error.ServiceUnknown")
                {
                    *guard = None;
                }
                None
            }
        }
    }

    pub fn refresh(out: &Out) -> Option<PlayerState> {
        let generation = {
            let s = SHARED.lock().unwrap();
            if !s.active {
                return None;
            }
            s.generation
        };
        let read = with_control(|c| {
            let owner = name_owner(c);
            let props = match owner {
                Some(_) => Some(props_of(&c.with_proxy(BUS, PATH, CALL).get_all(PLAYER)?)),
                None => None,
            };
            Ok((owner, props))
        });
        let state = {
            let mut s = SHARED.lock().unwrap();
            if !(s.active && s.generation == generation) {
                return None;
            }
            if let Some((owner, props)) = read {
                s.owner = owner.clone();
                s.state.running = owner.is_some();
                s.state.installed = owner.is_some() || installed();
                let now = now_ms();
                match props {
                    Some(props) => {
                        s.state.apply(&props, now);
                    }
                    None => s.state.clear(now),
                }
            }
            s.state.clone()
        };
        out.state(&state);
        Some(state)
    }

    pub fn control(out: &Out, action: &str, value: Option<f64>) -> bool {
        let (active, state) = {
            let s = SHARED.lock().unwrap();
            (s.active && s.owner.is_some(), s.state.clone())
        };
        if !active {
            return false;
        }
        let v = value.unwrap_or(0.0);
        let now = now_ms();
        let done = with_control(|c| {
            let p = c.with_proxy(BUS, PATH, CALL);
            match action {
                "playPause" => p.method_call(PLAYER, "PlayPause", ()),
                "next" => p.method_call(PLAYER, "Next", ()),
                "previous" => p.method_call(PLAYER, "Previous", ()),
                "seek" => {
                    let Some(track) = state.track.as_ref() else { return Err(dbus::Error::new_failed("no track")) };
                    let target = v.clamp(0.0, (track.duration - 1.0).max(0.0));
                    let target_us = (target * 1_000_000.0) as i64;
                    match dbus::Path::new(track.object_path.clone()) {
                        Ok(path) => p.method_call(PLAYER, "SetPosition", (path, target_us)),
                        Err(_) => {
                            let from = (state.position_at_time(now) * 1_000_000.0) as i64;
                            p.method_call(PLAYER, "Seek", (target_us - from,))
                        }
                    }
                }
                "shuffle" => p.set(PLAYER, "Shuffle", v != 0.0),
                "repeat" => p.set(PLAYER, "LoopStatus", if v != 0.0 { "Playlist" } else { "None" }),
                "volume" => p.set(PLAYER, "Volume", (v / 100.0).clamp(0.0, 1.0)),
                _ => Err(dbus::Error::new_failed("unknown action")),
            }
        })
        .is_some();
        if !done {
            return false;
        }
        let optimistic: Option<(String, Value)> = match action {
            "seek" => state.track.as_ref().map(|t| {
                let target = v.clamp(0.0, (t.duration - 1.0).max(0.0));
                ("Position".into(), Value::Int((target * 1_000_000.0) as i64))
            }),
            "shuffle" => Some(("Shuffle".into(), Value::Bool(v != 0.0))),
            "repeat" => Some(("LoopStatus".into(), Value::Str(if v != 0.0 { "Playlist" } else { "None" }.into()))),
            "volume" => Some(("Volume".into(), Value::Float((v / 100.0).clamp(0.0, 1.0)))),
            _ => None,
        };
        if let Some(prop) = optimistic {
            let state = {
                let mut s = SHARED.lock().unwrap();
                s.state.apply(&[prop], now_ms());
                s.state.clone()
            };
            out.state(&state);
        }
        true
    }

    fn launcher() -> Option<Vec<std::ffi::OsString>> {
        if let Some(bin) = crate::platform::find_on_path("feishin") {
            return Some(vec![bin.into()]);
        }
        let home = crate::platform::home_dir();
        let flatpak = [
            PathBuf::from("/var/lib/flatpak/app").join(FLATPAK_ID),
            home.join(".local/share/flatpak/app").join(FLATPAK_ID),
        ];
        if flatpak.iter().any(|p| p.is_dir()) {
            if let Some(bin) = crate::platform::find_on_path("flatpak") {
                return Some(vec![bin.into(), "run".into(), FLATPAK_ID.into()]);
            }
        }
        let appimage = home.join(".local/bin/feishin.AppImage");
        if appimage.is_file() {
            return Some(vec![appimage.into()]);
        }
        None
    }

    pub fn installed() -> bool {
        launcher().is_some()
    }

    pub fn open() -> bool {
        let raised = with_control(|c| {
            if name_owner(c).is_none() {
                return Ok(false);
            }
            let p = c.with_proxy(BUS, PATH, CALL);
            let can: bool = p.get(ROOT, "CanRaise").unwrap_or(true);
            if can {
                p.method_call::<(), _, _, _>(ROOT, "Raise", ())?;
            }
            Ok(can)
        })
        .unwrap_or(false);
        if raised {
            return true;
        }

        if let Some(argv) = launcher() {
            use std::os::unix::process::CommandExt;
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(0);
            match cmd.spawn() {
                Ok(mut child) => {
                    let _ = std::thread::Builder::new()
                        .name("coucou-feishin-child".into())
                        .spawn(move || {
                            let _ = child.wait();
                        });
                    return true;
                }
                Err(err) => crate::log::line(format!("feishin: could not start it: {err}")),
            }
        }
        crate::platform::open_url(DOWNLOAD_URL);
        false
    }

    fn want_artwork(app: &AppHandle, state: &PlayerState) {
        let Some(url) = state.track.as_ref().and_then(|t| t.art_url.clone()) else { return };
        {
            let mut art = ART.lock().unwrap();
            if let Some((_, data)) = art.entries.iter().find(|(u, _)| *u == url) {
                let _ = app.emit_to(
                    crate::island::WINDOW_LABEL,
                    ARTWORK_EVENT,
                    Artwork { art_url: url.clone(), data_url: data.clone() },
                );
                return;
            }
            if art.loading.as_deref() == Some(url.as_str()) {
                return;
            }
            art.loading = Some(url.clone());
        }
        let app = app.clone();
        match artwork_source(&url) {
            Some(ArtSource::Http(remote)) => {
                if crate::integrations::PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
                    ART.lock().unwrap().loading = None;
                    return;
                }
                tauri::async_runtime::spawn(async move {
                    let bytes = fetch(&remote).await;
                    finish_artwork(&app, url, bytes);
                });
            }
            Some(ArtSource::File(path)) => {
                tauri::async_runtime::spawn_blocking(move || {
                    let bytes = std::fs::metadata(&path)
                        .ok()
                        .filter(|m| m.is_file() && m.len() as usize <= ARTWORK_LIMIT)
                        .and_then(|_| std::fs::read(&path).ok());
                    finish_artwork(&app, url, bytes);
                });
            }
            None => {
                ART.lock().unwrap().loading = None;
            }
        }
    }

    async fn fetch(url: &str) -> Option<Vec<u8>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .ok()?;
        let response = client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        crate::net::read_capped(response, ARTWORK_LIMIT).await.ok()
    }

    fn finish_artwork(app: &AppHandle, url: String, bytes: Option<Vec<u8>>) {
        let data = bytes.as_deref().and_then(data_url);
        let mut art = ART.lock().unwrap();
        if art.loading.as_deref() == Some(url.as_str()) {
            art.loading = None;
        }
        let Some(data) = data else { return };
        art.entries.retain(|(u, _)| *u != url);
        art.entries.push_back((url.clone(), data.clone()));
        while art.entries.len() > 12 {
            art.entries.pop_front();
        }
        drop(art);
        let current = SHARED.lock().unwrap().state.track.as_ref().and_then(|t| t.art_url.clone());
        if current.as_deref() == Some(url.as_str()) {
            let _ = app.emit_to(crate::island::WINDOW_LABEL, ARTWORK_EVENT, Artwork { art_url: url, data_url: data });
        }
    }
}

// ── Windows Launcher ─────────────────────────────────────────────────────────

#[cfg(windows)]
mod windows {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    pub fn find_feishin() -> Option<PathBuf> {
        if let Some(bin) = crate::platform::find_on_path("Feishin.exe") {
            return Some(bin);
        }
        if let Some(bin) = crate::platform::find_on_path("feishin.exe") {
            return Some(bin);
        }
        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        if let Some(local) = local_app_data {
            let p = local.join("Programs").join("feishin").join("Feishin.exe");
            if p.is_file() {
                return Some(p);
            }
        }
        let prog_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
        if let Some(pf) = prog_files {
            let p = pf.join("Feishin").join("Feishin.exe");
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    pub fn installed() -> bool {
        find_feishin().is_some()
    }

    pub fn open() -> bool {
        if let Some(path) = find_feishin() {
            if Command::new(path).spawn().is_ok() {
                return true;
            }
        }
        crate::platform::open_url(DOWNLOAD_URL);
        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_feishin_metadata() {
        let mut map = Vec::new();
        map.push(("mpris:trackid".into(), Value::Str("/org/mpris/MediaPlayer2/track/abc".into())));
        map.push(("xesam:title".into(), Value::Str("Starboy".into())));
        map.push(("xesam:artist".into(), Value::Strs(vec!["The Weeknd".into(), "Daft Punk".into()])));
        map.push(("xesam:album".into(), Value::Str("Starboy".into())));
        // Feishin sending milliseconds: 230000 ms (230 seconds)
        map.push(("mpris:length".into(), Value::Int(230_000)));
        map.push(("mpris:artUrl".into(), Value::Str("http://127.0.0.1:4533/rest/getCoverArt?id=123".into())));

        let track = parse_metadata(&map).expect("metadata should parse");
        assert_eq!(track.id, "feishin:track:abc");
        assert_eq!(track.title, "Starboy");
        assert_eq!(track.artist, "The Weeknd, Daft Punk");
        assert_eq!(track.album, "Starboy");
        assert_eq!(track.duration, 230.0);
        assert_eq!(track.art_url.as_deref(), Some("http://127.0.0.1:4533/rest/getCoverArt?id=123"));
    }

    #[test]
    fn parses_duration_in_microseconds() {
        let mut map = Vec::new();
        map.push(("xesam:title".into(), Value::Str("Song".into())));
        // Standard microseconds: 240,000,000 us (240s)
        map.push(("mpris:length".into(), Value::Int(240_000_000)));
        let track = parse_metadata(&map).expect("track");
        assert_eq!(track.duration, 240.0);
    }

    #[test]
    fn parses_artwork_sources() {
        assert_eq!(
            artwork_source("http://localhost:4533/cover.jpg"),
            Some(ArtSource::Http("http://localhost:4533/cover.jpg".into()))
        );
        assert_eq!(
            artwork_source("https://subsonic.example.com/cover.jpg"),
            Some(ArtSource::Http("https://subsonic.example.com/cover.jpg".into()))
        );
        assert_eq!(
            artwork_source("file:///home/user/Music/cover.jpg"),
            Some(ArtSource::File(std::path::PathBuf::from("/home/user/Music/cover.jpg")))
        );
    }
}
