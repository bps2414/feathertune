// Audio: one Web Audio graph on the page's own <video>, shared by every audio plugin (audio.rs).
//
// - The graph is created on the first `play` of a <video>, once per element, in one AudioContext:
//   video → input → [effects] → trim → master → destination. A node that is neutral is left out
//   of the chain (it is kept, and reconnected when it is needed again).
// - createMediaElementSource cannot be undone: from then on the element only sounds through the
//   graph (and the site reuses the same element for every track). So the path to `destination` is
//   never cut: connections are changed by adding the new ones before removing the old ones, and
//   every bypass happens inside the graph.
// - While an ad plays every parameter goes back to neutral in 30 ms and nothing is scheduled,
//   measured or skipped. No player at all counts as an ad.
// - Failure guard: if the context will not run, or the graph passes only digital silence while the
//   video plays, the module marks the tab session (sessionStorage) and reloads the page once; with
//   the mark it creates no graph. Losing the effects is better than losing the sound.
// - Every curve and ramp is scheduled on the audio clock (AudioParam), never with JS timers, which
//   Chromium holds back while the window is hidden.
if (window.top !== window) return;

// Every selector and site-state read lives here (not verified from the repository; see DESIGN.md).
const SELECTORS = {
  player: '#movie_player',
  // Its getState() is the site's state: `player.adPlaying`, `queue`.
  playerBar: 'ytmusic-player-bar',
};
const OFF_KEY = 'feathertune-audio-off';
const RAMP = 0.03;
// The context is suspended after this long paused, and resumed on `play`.
const SUSPEND_MS = 10000;
// Failure guard: digital silence for GUARD_ZERO_S after GUARD_FROM_S of a track trips it.
const GUARD_FROM_S = 5;
const GUARD_ZERO_S = 4;
// The guard only looks until it has heard sound once through the graph.
const GUARD_UNTIL_S = 60;

const off = (() => {
  try {
    return sessionStorage.getItem(OFF_KEY) === '1';
  } catch {
    return false;
  }
})();

let data = null;
let ctx = null;
let nodes = null;
// The connections in place, as [from, to] pairs.
let edges = [];
const sources = new WeakMap();
let video = null;
let ad = true;
let suspendTimer = 0;
let worklet = 'untested';
const guard = { done: false, zeroFrom: null, resumeAt: null };

// --- site state ---

const siteState = () => {
  try {
    return document.querySelector(SELECTORS.playerBar)?.getState?.() ?? null;
  } catch {
    return null;
  }
};
const adNow = () => {
  const player = document.querySelector(SELECTORS.player);
  if (!player) return true;
  if (player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting')) return true;
  return siteState()?.player?.adPlaying === true;
};
const on = (plugin) => !!data?.plugins?.[plugin];
const anyOn = () => !!data && Object.values(data.plugins || {}).some(Boolean);

// --- parameters ---

// Moves a parameter to `value` in about 30 ms from where it is now, dropping what was scheduled.
const ramp = (param, value) => {
  const t = ctx.currentTime;
  param.cancelAndHoldAtTime(t);
  param.setTargetAtTime(value, t, RAMP / 3);
};
const dbToGain = (db) => Math.pow(10, db / 20);

// --- the graph ---

// The chain from `input` to `destination` for the current settings and ad state.
const chain = () => {
  const list = [nodes.input];
  if (!ad) list.push(...effects());
  list.push(nodes.trim, nodes.master, ctx.destination);
  return list;
};
// The effect nodes in use, in order (filled in by the effects below).
const effects = () => [];
// Side branches (taps) that must be connected now: [from, to] pairs.
const taps = () => {
  const list = [];
  if (analyserUsers.size) list.push([nodes.input, nodes.analyser]);
  return list;
};

const wire = () => {
  if (!nodes) return;
  const list = chain();
  const want = [];
  for (let i = 0; i < list.length - 1; i++) want.push([list[i], list[i + 1]]);
  want.push(...taps());
  const has = (set, [a, b]) => set.some(([x, y]) => x === a && y === b);
  // New connections first, so the sound always has a path.
  for (const e of want) if (!has(edges, e)) e[0].connect(e[1]);
  for (const e of edges) {
    if (has(want, e)) continue;
    try {
      e[0].disconnect(e[1]);
    } catch {}
  }
  edges = want;
};

// Every parameter to what the settings ask (or neutral during an ad), in 30 ms.
const apply = () => {
  if (!nodes) return;
  wire();
  ramp(nodes.trim.gain, ad ? 1 : trimGain());
  if (ad) ramp(nodes.master.gain, 1);
};
// The trim's gain: make-up and pre-gain of the effects (filled in below).
const trimGain = () => 1;

// The analyser is connected only while something reads from it.
const analyserUsers = new Set();
const useAnalyser = (user, wanted) => {
  if (wanted === analyserUsers.has(user)) return;
  if (wanted) analyserUsers.add(user);
  else analyserUsers.delete(user);
  wire();
};
const level = (() => {
  let buffer = null;
  // The RMS of the last few ms at the input, in dBFS (-Infinity: digital silence).
  return () => {
    buffer ||= new Float32Array(nodes.analyser.fftSize);
    nodes.analyser.getFloatTimeDomainData(buffer);
    let sum = 0;
    for (const x of buffer) sum += x * x;
    return sum ? 10 * Math.log10(sum / buffer.length) + 3.01 : -Infinity;
  };
})();

const createGraph = (element) => {
  if (!ctx) {
    ctx = new AudioContext({ latencyHint: 'playback' });
    nodes = {
      input: ctx.createGain(),
      trim: ctx.createGain(),
      master: ctx.createGain(),
      analyser: ctx.createAnalyser(),
    };
    nodes.analyser.fftSize = 2048;
    testWorklet();
  }
  if (!sources.has(element)) {
    const source = ctx.createMediaElementSource(element);
    // Never disconnected: every element the site uses feeds the same input.
    source.connect(nodes.input);
    sources.set(element, source);
  }
  apply();
};

// Whether an AudioWorklet module from a Blob loads under the site's CSP (the probe reports it).
const testWorklet = () => {
  if (!ctx.audioWorklet) {
    worklet = 'missing';
    return;
  }
  const code = 'registerProcessor("feathertune-probe", class extends AudioWorkletProcessor { process() { return false; } });';
  const url = URL.createObjectURL(new Blob([code], { type: 'application/javascript' }));
  ctx.audioWorklet
    .addModule(url)
    .then(() => (worklet = 'ok'))
    .catch(() => (worklet = 'blocked'))
    .finally(() => URL.revokeObjectURL(url));
};

const trip = (why) => {
  console.warn(`[feathertune] audio: ${why}; the graph is off until this tab is closed, reloading once`);
  try {
    sessionStorage.setItem(OFF_KEY, '1');
  } catch {
    return;
  }
  location.reload();
};

// The failure guard, on the video's own timeupdate (about 4 a second).
const checkGuard = (v) => {
  if (guard.done || !ctx) return;
  const playing = !v.paused && !v.muted && v.volume > 0 && v.readyState >= 3;
  if (!playing || ad) {
    guard.zeroFrom = null;
    useAnalyser('guard', false);
    return;
  }
  if (ctx.state !== 'running') {
    if (guard.resumeAt === null) {
      guard.resumeAt = v.currentTime;
      ctx.resume().catch(() => {});
    } else if (v.currentTime - guard.resumeAt > 1.5) {
      trip('the audio context does not run');
    }
    return;
  }
  guard.resumeAt = null;
  if (v.currentTime < GUARD_FROM_S || v.currentTime > GUARD_UNTIL_S) {
    useAnalyser('guard', false);
    guard.zeroFrom = null;
    return;
  }
  useAnalyser('guard', true);
  if (level() > -Infinity) {
    guard.done = true;
    useAnalyser('guard', false);
    return;
  }
  guard.zeroFrom ??= v.currentTime;
  if (v.currentTime - guard.zeroFrom >= GUARD_ZERO_S) trip('only silence passes through the graph');
};

// --- ads ---

let observed = null;
const adObserver = new MutationObserver(() => setAd(adNow()));
const watchPlayer = () => {
  const player = document.querySelector(SELECTORS.player);
  if (player && player !== observed) {
    adObserver.disconnect();
    adObserver.observe(player, { attributes: true, attributeFilter: ['class'] });
    observed = player;
  }
  setAd(adNow());
};
const setAd = (now) => {
  if (now === ad) return;
  ad = now;
  onAd(now);
  apply();
};
// What each effect does when an ad starts or ends (filled in below).
const onAd = () => {};

// --- media events ---

const onMedia = (e) => {
  const v = e.target;
  if (!(v instanceof HTMLVideoElement)) return;
  watchPlayer();
  if (off) return;
  if (e.type === 'play' && anyOn()) {
    video = v;
    createGraph(v);
  }
  if (!ctx || !sources.has(v)) return;
  video = v;
  switch (e.type) {
    case 'play':
      clearTimeout(suspendTimer);
      if (ctx.state === 'suspended') ctx.resume().catch(() => {});
      break;
    case 'pause':
      clearTimeout(suspendTimer);
      suspendTimer = setTimeout(() => video?.paused && ctx.suspend().catch(() => {}), SUSPEND_MS);
      break;
    case 'timeupdate':
      checkGuard(v);
      break;
  }
};
for (const type of ['play', 'pause', 'playing', 'timeupdate', 'loadstart', 'loadedmetadata', 'seeking', 'seeked', 'ratechange', 'ended']) {
  document.addEventListener(type, onMedia, true);
}

feathertune.audio = Object.freeze({
  set: (d) => {
    data = d;
    if (off) return;
    // Settings arrived after the first play: start now on the playing element.
    const playing = [...document.querySelectorAll('video')].find((v) => !v.paused);
    if (!ctx && playing && anyOn()) {
      video = playing;
      watchPlayer();
      createGraph(playing);
    }
    apply();
  },
  // --measure: one line of state.
  probe: () => {
    const state = off ? 'off' : ctx ? ctx.state : 'none';
    return `audio=${state} ctx_ms=${ctx ? Math.round(ctx.currentTime * 1000) : 0} worklet=${worklet} ad=${ad}`;
  },
});
