// The Feishin pill and card, Mochi's dance, and state handling.

import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { emit, sent } from "./tauri.mjs";
import { installFakeDom } from "./fakedom.mjs";
import {
  IDLE_FEISHIN, FEISHIN_ID, Feishin, currentArtwork, desktopDances, formatTime, islandDances,
  feishinPlaying, feishinPosition, volumeLevel, withPlaying,
} from "../src/core/feishin.ts";
import { BotEngine, danceTransform, stepDanceLevel } from "../src/mochi/engine.ts";
import { registerFeishinHandlers } from "../src/island/feishin.ts";
import { buildFeishinCard, buildFeishinPill } from "../src/views/feishin.ts";
import { DEFAULT_SETTINGS, State } from "../src/core/state.ts";
import { lookup, setLanguage } from "../src/i18n/i18n.ts";

installFakeDom();

const track = (over = {}) => ({
  id: "feishin:track:abc", title: "Starboy", artist: "The Weeknd, Daft Punk", album: "Starboy",
  duration: 230, artUrl: "http://127.0.0.1:4533/rest/getCoverArt?id=123", ...over,
});

const playing = (over = {}) => ({
  ...IDLE_FEISHIN, running: true, installed: true, track: track(), playing: true,
  position: 15, positionAt: 1_000, volume: 80, ...over,
});

// ── Position ──────────────────────────────────────────────────────────────────

test("feishin: the position runs on from its timestamp while playing, and stops at the end", () => {
  const s = playing();
  assert.equal(feishinPosition(s, 1_000), 15);
  assert.equal(feishinPosition(s, 3_500), 17.5);
  assert.equal(feishinPosition(s, 0), 15, "a clock behind the anchor never moves it back");
  assert.equal(feishinPosition(s, 1_000_000), 230, "capped at track length");
  assert.equal(feishinPosition({ ...s, playing: false }, 60_000), 15, "paused stays put");
  assert.equal(feishinPosition({ ...s, track: null }, 3_000), 17, "no length, no cap");
});

test("feishin: a play/pause click freezes or restarts the clock where it is", () => {
  const paused = withPlaying(playing(), false, 5_000);
  assert.equal(paused.playing, false);
  assert.equal(paused.position, 19);
  assert.equal(paused.positionAt, 5_000);
  assert.equal(feishinPosition(paused, 9_000), 19);
  const again = withPlaying(paused, true, 9_000);
  assert.equal(feishinPosition(again, 10_000), 20);
  const same = playing();
  assert.equal(withPlaying(same, true, 99), same, "no change, same object");
});

test("feishin: times format m:ss and h:mm:ss", () => {
  assert.equal(formatTime(0), "0:00");
  assert.equal(formatTime(65.9), "1:05");
  assert.equal(formatTime(599), "9:59");
  assert.equal(formatTime(3725), "1:02:05");
});

test("feishin: volume levels 0..3", () => {
  assert.equal(volumeLevel(0), 0);
  assert.equal(volumeLevel(20), 1);
  assert.equal(volumeLevel(50), 2);
  assert.equal(volumeLevel(85), 3);
});

test("feishin: artwork shows only on the track it belongs to", () => {
  const s = playing();
  Feishin.artwork = { artUrl: "http://127.0.0.1:4533/rest/getCoverArt?id=123", dataUrl: "data:image/jpeg;base64,AA" };
  assert.equal(currentArtwork(s), "data:image/jpeg;base64,AA");
  assert.equal(currentArtwork({ ...s, track: { ...s.track, artUrl: "http://other" } }), null);
  Feishin.artwork = null;
  assert.equal(currentArtwork(s), null);
});

test("feishin: music counts only when it plays on a declared Feishin pill", () => {
  const s = playing();
  assert.ok(feishinPlaying(s, [FEISHIN_ID, "integration_n8n"]));
  assert.ok(!feishinPlaying({ ...s, playing: false }, [FEISHIN_ID]));
  assert.ok(!feishinPlaying({ ...s, track: null }, [FEISHIN_ID]));
  assert.ok(!feishinPlaying(s, ["integration_spotify"]));
});

test("feishin: Mochi dances when music plays and Feishin is focused or compact", () => {
  assert.ok(islandDances({ music: true, state: "idle", mode: "compact", view: "overview", focusId: null }));
  assert.ok(islandDances({ music: true, state: "working", mode: "compact", view: "overview", focusId: null }));
  assert.ok(islandDances({ music: true, state: "idle", mode: "expanded", view: "overview", focusId: FEISHIN_ID }));
  assert.ok(!islandDances({ music: true, state: "idle", mode: "expanded", view: "overview", focusId: "other" }));
  assert.ok(!islandDances({ music: false, state: "idle", mode: "compact", view: "overview", focusId: null }));
  assert.ok(desktopDances(true, "idle"));
  assert.ok(!desktopDances(false, "idle"));
});

// ── Views ─────────────────────────────────────────────────────────────────────

test("feishin: the idle card and playing card render correctly", () => {
  const card = buildFeishinCard();
  Feishin.state = { ...IDLE_FEISHIN, installed: true };
  card.sync();
  assert.match(card.el.textContent, /Feishin.*Integration.*Not playing.*Open Feishin/);

  Feishin.state = playing();
  card.sync();
  assert.match(card.el.textContent, /Starboy/);
  assert.match(card.el.textContent, /The Weeknd, Daft Punk/);
});

test("feishin: the pill shows track title and controls on hover", () => {
  Feishin.state = playing();
  const task = { id: FEISHIN_ID, name: "Starboy", color: "#3574FC", state: "idle", steps: [], source: "n8n", isIntegration: true, stepIndex: 0 };
  const pill = buildFeishinPill(task, () => {});
  pill.sync();
  assert.equal(pill.el.querySelector(".lbl").textContent, "Starboy");
  assert.ok(pill.el.querySelector(".np-pill-controls"));
});
