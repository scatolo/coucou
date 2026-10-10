// Feishin reports → the page:
// Linux: via MPRIS over D-Bus (src-tauri/src/feishin.rs).
// Windows: via Feishin's native Remote WebSocket server (ws://127.0.0.1:4333).

import { Bridge, onEvent } from "../core/bridge";
import { pillDefinition } from "../core/pills";
import { FEISHIN_ID, Feishin, type FeishinState, type FeishinTrack } from "../core/feishin";
import { State } from "../core/state";
import { t } from "../i18n/i18n";

export interface FeishinHost {
  /** Compact island from hidden, no peek sound. */
  revealSilently(): void;
}

let wsClient: WebSocket | null = null;
let wsReconnectTimer: number | null = null;
let activeIsland: FeishinHost | null = null;

export function registerFeishinHandlers(island: FeishinHost) {
  activeIsland = island;

  // Linux MPRIS events from Rust
  void onEvent<FeishinState>("feishin", (s) => applyFeishin(island, s));
  void onEvent<{ artUrl: string; dataUrl: string }>("feishin-artwork", (art) => {
    Feishin.artwork = art;
    State.notify();
  });

  State.subscribe(() => {
    syncPillName();
    refreshWhenShown();
    syncWebSocketConnection();
  });

  syncWebSocketConnection();
}

/** A report from Rust (or the answer to a refresh). */
export function applyFeishin(island: FeishinHost, next: FeishinState) {
  const wasPlaying = State.feishinPlaying;
  Feishin.state = next;
  syncPillName();
  // Only on not playing → playing.
  if (!wasPlaying && State.feishinPlaying && !State.paused && State.mode === "hidden") {
    island.revealSilently();
  }
  State.notify();
}

/** Syncs the pill name with the track title, or "Feishin". */
export function syncPillName() {
  const task = State.tasks.find((x) => x.id === FEISHIN_ID);
  if (!task) return;
  const track = Feishin.state.track;
  const name = track?.title || (pillDefinition(FEISHIN_ID)?.name ?? "Feishin");
  if (task.name !== name) task.name = name;
}

let shown = false;

function refreshWhenShown() {
  const now = State.mode === "expanded" && State.view === "overview" && State.focusTask?.id === FEISHIN_ID;
  if (now && !shown) void Bridge.feishinRefresh();
  shown = now;
}

// ── Windows / Direct WebSocket Client (Feishin Remote) ────────────────────────

function isFeishinDeclared(): boolean {
  return State.settings.activeIntegrations.includes(FEISHIN_ID);
}

function syncWebSocketConnection() {
  // Only maintain WebSocket connection if Feishin pill is active.
  // On Windows, WebSocket is the primary communication channel.
  // On Linux, MPRIS is primary, but WebSocket also works seamlessly if available.
  const shouldConnect = isFeishinDeclared();
  if (!shouldConnect) {
    disconnectWebSocket();
    return;
  }

  if (wsClient == null && wsReconnectTimer == null) {
    connectWebSocket();
  }
}

function disconnectWebSocket() {
  if (wsReconnectTimer != null) {
    window.clearTimeout(wsReconnectTimer);
    wsReconnectTimer = null;
  }
  if (wsClient) {
    try {
      wsClient.close();
    } catch {
      // ignore
    }
    wsClient = null;
  }
}

function connectWebSocket() {
  if (!isFeishinDeclared()) return;

  try {
    const ws = new WebSocket("ws://127.0.0.1:4333");
    wsClient = ws;

    ws.onopen = () => {
      if (wsReconnectTimer != null) {
        window.clearTimeout(wsReconnectTimer);
        wsReconnectTimer = null;
      }
      Feishin.state = {
        ...Feishin.state,
        running: true,
        installed: true,
      };
      State.notify();
    };

    ws.onmessage = (event) => {
      try {
        const payload = JSON.parse(event.data);
        handleRemoteEvent(payload);
      } catch (err) {
        console.error("[coucou] Feishin WS parse error:", err);
      }
    };

    ws.onclose = () => {
      wsClient = null;
      // If we are on Windows (no D-Bus fallback), mark not running when disconnected
      if (typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent ?? "")) {
        Feishin.state = {
          ...Feishin.state,
          running: false,
          playing: false,
          track: null,
        };
        State.notify();
      }
      scheduleReconnect();
    };

    ws.onerror = () => {
      try {
        ws.close();
      } catch {
        // ignore
      }
    };
  } catch {
    scheduleReconnect();
  }
}

function scheduleReconnect() {
  if (wsReconnectTimer != null || !isFeishinDeclared()) return;
  wsReconnectTimer = window.setTimeout(() => {
    wsReconnectTimer = null;
    connectWebSocket();
  }, 3500);
}

function handleRemoteEvent(msg: { event: string; data: any }) {
  if (!msg || !msg.event) return;

  switch (msg.event) {
    case "state": {
      const data = msg.data || {};
      const song = data.song;
      const status = data.status; // "PLAYING", "PAUSED", "STOPPED"
      const playing = status === "PLAYING";
      const track = song ? formatRemoteSong(song) : null;
      const wasPlaying = State.feishinPlaying;

      Feishin.state = {
        ...Feishin.state,
        running: true,
        installed: true,
        track,
        playing,
        position: 0,
        positionAt: Date.now(),
        shuffle: Boolean(data.shuffle),
        repeat: data.repeat != null && data.repeat !== "NONE",
        volume: Number.isFinite(data.volume) ? Number(data.volume) : 50,
      };

      if (track?.artUrl && wsClient && wsClient.readyState === WebSocket.OPEN) {
        wsClient.send(JSON.stringify({ event: "proxy" }));
      }

      syncPillName();
      if (!wasPlaying && State.feishinPlaying && !State.paused && State.mode === "hidden" && activeIsland) {
        activeIsland.revealSilently();
      }
      State.notify();
      break;
    }

    case "playback": {
      const status = msg.data;
      const playing = status === "PLAYING";
      const wasPlaying = State.feishinPlaying;
      Feishin.state = {
        ...Feishin.state,
        playing,
        positionAt: Date.now(),
      };
      if (!wasPlaying && State.feishinPlaying && !State.paused && State.mode === "hidden" && activeIsland) {
        activeIsland.revealSilently();
      }
      State.notify();
      break;
    }

    case "song": {
      const song = msg.data;
      const track = song ? formatRemoteSong(song) : null;
      Feishin.state = {
        ...Feishin.state,
        track,
        position: 0,
        positionAt: Date.now(),
      };
      if (track?.artUrl && wsClient && wsClient.readyState === WebSocket.OPEN) {
        wsClient.send(JSON.stringify({ event: "proxy" }));
      }
      syncPillName();
      State.notify();
      break;
    }

    case "volume": {
      const vol = Number(msg.data);
      if (Number.isFinite(vol)) {
        Feishin.state = { ...Feishin.state, volume: Math.round(vol) };
        State.notify();
      }
      break;
    }

    case "shuffle": {
      Feishin.state = { ...Feishin.state, shuffle: Boolean(msg.data) };
      State.notify();
      break;
    }

    case "repeat": {
      const rep = msg.data;
      Feishin.state = { ...Feishin.state, repeat: rep != null && rep !== "NONE" };
      State.notify();
      break;
    }

    case "proxy": {
      // Base64 cover art
      const base64 = msg.data;
      if (base64 && typeof base64 === "string" && Feishin.state.track?.artUrl) {
        const dataUrl = `data:image/jpeg;base64,${base64}`;
        Feishin.artwork = { artUrl: Feishin.state.track.artUrl, dataUrl };
        State.notify();
      }
      break;
    }
  }
}

function formatRemoteSong(song: any): FeishinTrack {
  const artists = Array.isArray(song.artists)
    ? song.artists.map((a: any) => (typeof a === "string" ? a : a.name)).filter(Boolean).join(", ")
    : (song.artist ?? "");

  return {
    id: `feishin:track:${song.id ?? "unknown"}`,
    title: song.name ?? song.title ?? "",
    artist: artists,
    album: song.album ?? "",
    duration: Number.isFinite(song.duration) ? Number(song.duration) : 0,
    artUrl: song.imageUrl ?? null,
  };
}

// ── Control Dispatcher ────────────────────────────────────────────────────────

export function sendFeishinControl(action: string, value?: number) {
  // If WebSocket is active (Windows or connected local server), send over WS
  if (wsClient && wsClient.readyState === WebSocket.OPEN) {
    switch (action) {
      case "playPause":
        wsClient.send(JSON.stringify({ event: Feishin.state.playing ? "pause" : "play" }));
        break;
      case "next":
        wsClient.send(JSON.stringify({ event: "next" }));
        break;
      case "previous":
        wsClient.send(JSON.stringify({ event: "previous" }));
        break;
      case "seek":
        if (value != null) {
          wsClient.send(JSON.stringify({ event: "position", position: value }));
        }
        break;
      case "shuffle":
        wsClient.send(JSON.stringify({ event: "shuffle" }));
        break;
      case "repeat":
        wsClient.send(JSON.stringify({ event: "repeat" }));
        break;
      case "volume":
        if (value != null) {
          wsClient.send(JSON.stringify({ event: "volume", volume: value }));
        }
        break;
    }
  }

  // Also notify Rust (for Linux MPRIS)
  void Bridge.feishinControl(action as any, value);
}
