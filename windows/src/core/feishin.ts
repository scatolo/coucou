// The Feishin pill on the page: what src-tauri/src/feishin.rs reports (Linux via MPRIS,
// Windows via Remote WebSocket), and the pure rules the views and Mochi follow.

import type { BotStateName, IslandMode, IslandViewName } from "./layout";

export const FEISHIN_ID = "integration_feishin";
/** Feishin brand accent blue. */
export const FEISHIN_BLUE = "#3574FC";

export interface FeishinTrack {
  id: string;
  title: string;
  artist: string;
  album: string;
  /** Seconds. */
  duration: number;
  artUrl: string | null;
}

export interface FeishinState {
  /** Feishin is running. */
  running: boolean;
  /** Feishin is installed / accessible to launch. */
  installed: boolean;
  track: FeishinTrack | null;
  playing: boolean;
  /** Seconds at `positionAt` (Unix ms); while playing it runs on from there. */
  position: number;
  positionAt: number;
  shuffle: boolean;
  repeat: boolean;
  /** 0…100. */
  volume: number;
}

export const IDLE_FEISHIN: FeishinState = {
  running: false,
  installed: false,
  track: null,
  playing: false,
  position: 0,
  positionAt: 0,
  shuffle: false,
  repeat: false,
  volume: 50,
};

/** The page's copy of the player, and the cover of the track that has one. */
export const Feishin = {
  state: { ...IDLE_FEISHIN } as FeishinState,
  artwork: null as { artUrl: string; dataUrl: string } | null,
};

/** The cover to show for the current track, if it has arrived. */
export function currentArtwork(s: FeishinState = Feishin.state): string | null {
  const art = Feishin.artwork;
  return art && s.track?.artUrl && art.artUrl === s.track.artUrl ? art.dataUrl : null;
}

/** Feishin position extrapolated while playing, capped at the end. */
export function feishinPosition(s: FeishinState, nowMs: number): number {
  const elapsed = s.playing ? Math.max(0, (nowMs - s.positionAt) / 1000) : 0;
  const p = s.position + elapsed;
  const d = s.track?.duration ?? 0;
  return d > 0 ? Math.min(p, d) : p;
}

/** The clock frozen or restarted where it is now. */
export function withPlaying(s: FeishinState, playing: boolean, nowMs: number): FeishinState {
  if (s.playing === playing) return s;
  return { ...s, position: feishinPosition(s, nowMs), positionAt: nowMs, playing };
}

/** NowPlayingProgress.format: m:ss, or h:mm:ss past an hour. */
export function formatTime(seconds: number): string {
  const s = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  const pad = (n: number) => String(n).padStart(2, "0");
  return s >= 3600
    ? `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`
    : `${Math.floor(s / 60)}:${pad(s % 60)}`;
}

/** NowPlayingVolume.icon: 0 = muted, then one, two or three waves. */
export function volumeLevel(volume: number): 0 | 1 | 2 | 3 {
  if (volume <= 0) return 0;
  if (volume < 34) return 1;
  if (volume < 67) return 2;
  return 3;
}

/** Music is playing on a declared Feishin pill. */
export function feishinPlaying(s: FeishinState, activeIntegrations: readonly string[]): boolean {
  return s.playing && s.track != null && activeIntegrations.includes(FEISHIN_ID);
}

const DANCE_STATES: ReadonlySet<BotStateName> = new Set(["idle", "working", "thinking", "searching", "finished"]);

export function islandDances(o: {
  music: boolean;
  state: BotStateName;
  mode: IslandMode;
  view: IslandViewName;
  focusId: string | null | undefined;
}): boolean {
  if (!o.music || !DANCE_STATES.has(o.state)) return false;
  if (o.mode === "compact") return true;
  return o.mode === "expanded" && o.view === "overview" && o.focusId === FEISHIN_ID;
}

export function desktopDances(music: boolean, state: BotStateName): boolean {
  return music && DANCE_STATES.has(state);
}
