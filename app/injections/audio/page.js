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

// The chain from `input` to `destination`: the effect nodes in use, in their fixed order.
const chain = () => [nodes.input, ...inChain, nodes.trim, nodes.master, ctx.destination];
// Effect nodes in the chain now, and the ones on their way out (ramping to neutral first).
let inChain = [];
const leaving = new Set();
let settleTimer = 0;
// Side branches (taps) that must be connected now: [from, to] pairs.
const taps = () => {
  const list = [];
  if (analyserUsers.size) list.push([nodes.input, nodes.analyser]);
  if (echo.on) {
    list.push([nodes.trim, nodes.echoSend], [nodes.echoSend, nodes.delay], [nodes.delay, nodes.feedback]);
    list.push([nodes.feedback, nodes.delay], [nodes.delay, nodes.echoWet], [nodes.echoWet, ctx.destination]);
  }
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

// --- equalizer and compressor ---

// 10 bands, 31 Hz to 16 kHz: shelves at the ends, peaking filters between.
const EQ_FREQUENCIES = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_Q = 1.4;
// The compressor presets. Neutral (out of the chain) is threshold 0 dB, ratio 1.
const COMPRESSORS = {
  // Gentle glue: only the loudest peaks are pulled down a little.
  soft: { threshold: -18, knee: 12, ratio: 2, attack: 0.01, release: 0.25, makeup: 2 },
  // Quiet listening: loud passages come down a lot, so quiet ones can be turned up.
  night: { threshold: -30, knee: 10, ratio: 5, attack: 0.003, release: 0.3, makeup: 6 },
  // Strong levelling, close to a limiter.
  strong: { threshold: -24, knee: 4, ratio: 10, attack: 0.001, release: 0.12, makeup: 5 },
};

const eqGains = () => (on('equalizer') && Array.isArray(data?.eq) ? data.eq : []);
const compressor = () => (on('compressor') ? COMPRESSORS[data?.compressor] : null);

// The effect nodes the settings want now, in chain order (none during an ad).
const wanted = () => {
  if (ad) return [];
  const list = [];
  eqGains().forEach((g, i) => g && list.push(nodes.eq[i]));
  if (compressor()) list.push(nodes.compressor);
  return list;
};
// The fixed order of every effect node.
const order = () => [...nodes.eq, nodes.compressor];

const neutralize = (node) => {
  if (node === nodes.compressor) {
    ramp(node.threshold, 0);
    ramp(node.ratio, 1);
  } else {
    ramp(node.gain, 0);
  }
};
const target = (node) => {
  if (node === nodes.compressor) {
    const c = compressor();
    for (const k of ['threshold', 'knee', 'ratio', 'attack', 'release']) ramp(node[k], c[k]);
  } else {
    ramp(node.gain, eqGains()[nodes.eq.indexOf(node)]);
  }
};

// The trim's gain: the equalizer's pre-gain (minus its largest boost, so it never clips) plus the
// compressor's make-up.
const trimGain = () => {
  if (ad) return 1;
  const boost = Math.max(0, ...eqGains());
  return dbToGain(-boost + (compressor()?.makeup ?? 0));
};

// Every parameter to what the settings ask (or neutral during an ad), in 30 ms. A node that is no
// longer wanted ramps to neutral first and leaves the chain 100 ms later.
const apply = () => {
  if (!nodes) return;
  const want = new Set(wanted());
  for (const n of inChain) {
    if (!want.has(n)) {
      neutralize(n);
      leaving.add(n);
    }
  }
  for (const n of want) leaving.delete(n);
  inChain = order().filter((n) => want.has(n) || leaving.has(n));
  wire();
  for (const n of want) target(n);
  ramp(nodes.trim.gain, trimGain());
  if (ad) ramp(nodes.master.gain, 1);
  if (leaving.size) {
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => {
      inChain = inChain.filter((n) => !leaving.has(n));
      leaving.clear();
      wire();
    }, 100);
  }
};

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
      eq: EQ_FREQUENCIES.map((f, i) => {
        const band = ctx.createBiquadFilter();
        band.type = i === 0 ? 'lowshelf' : i === EQ_FREQUENCIES.length - 1 ? 'highshelf' : 'peaking';
        band.frequency.value = f;
        band.Q.value = EQ_Q;
        band.gain.value = 0;
        return band;
      }),
      compressor: ctx.createDynamicsCompressor(),
      // The echo on a fading tail: trim → send → delay ⇄ feedback, delay → wet → destination.
      echoSend: ctx.createGain(),
      delay: ctx.createDelay(2),
      feedback: ctx.createGain(),
      echoWet: ctx.createGain(),
    };
    nodes.echoSend.gain.value = 0;
    nodes.feedback.gain.value = ECHO_FEEDBACK;
    nodes.echoWet.gain.value = ECHO_WET;
    nodes.analyser.fftSize = 2048;
    nodes.compressor.threshold.value = 0;
    nodes.compressor.ratio.value = 1;
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

// --- crossfade (one player) ---
//
// At the `out` point (the track's end minus the fade, by default) `master` goes down along the
// curve; when the fade is over the module seeks to `duration - 0.05` so the track ends by itself
// and the site picks the next one (repeat, shuffle and autoplay stay the site's). The next track
// fades in only after a fade-out (so a track started any other way never dips). The fade is armed
// from the video's own timeupdate (about 4 a second) and scheduled on the audio clock; it is
// rescheduled on pause, seek and rate changes. Nothing is scheduled during an ad.

const CURVE_POINTS = 256;
const ARM_BEFORE_S = 1;
const SKIP_FADE_IN_S = 0.3;
const ECHO_FEEDBACK = 0.45;
const ECHO_WET = 0.5;
const ECHO_DEFAULT_S = 0.375;
// How long the echo keeps ringing after the fade.
const ECHO_RING_S = 5;

// Fade-out curves, from 1 to 0 (the fade-in is the same curve backwards).
const CURVES = (() => {
  const make = (f) => Float32Array.from({ length: CURVE_POINTS }, (_, i) => f(i / (CURVE_POINTS - 1)));
  return {
    linear: make((t) => 1 - t),
    equal_power: make((t) => Math.cos((t * Math.PI) / 2)),
    s_curve: make((t) => 0.5 * (1 + Math.cos(Math.PI * t))),
    exponential: make((t) => (Math.pow(10, -3 * t) - 0.001) / 0.999),
  };
})();
const FADE_IN = Object.fromEntries(Object.entries(CURVES).map(([k, c]) => [k, Float32Array.from(c).reverse()]));

// The curve's value at p (0..1) and the part of it from p to the end.
const valueAt = (curve, p) => curve[Math.min(curve.length - 1, Math.round(p * (curve.length - 1)))];
const slice = (curve, p) => {
  if (p <= 0) return curve;
  const n = Math.max(2, Math.round((1 - p) * (curve.length - 1)) + 1);
  return Float32Array.from({ length: n }, (_, i) => valueAt(curve, p + ((1 - p) * i) / (n - 1)));
};

let trackId = null;
// The analysis of the current and next track (from Rust; see track_analysis).
const facts = () => data?.facts?.current ?? null;
// The fade of the current track: { out, dur, curve } in video seconds, or null.
let fade = null;
let armed = false;
// The fade-out reached its end (or was cut short by a track change).
let faded = false;
let ownSeek = false;
const echo = { on: false, until: 0 };

const currentId = () => {
  const player = document.querySelector(SELECTORS.player);
  return player?.getVideoData?.()?.video_id || new URLSearchParams(location.search).get('v');
};
const repeatOne = (v) => v.loop || siteState()?.queue?.repeatMode === 'ONE';
const seekTo = (v, t) => {
  ownSeek = true;
  v.currentTime = t;
};

// The fade for this track from the global settings (a mix replaces it, see below).
const planFade = (v) => {
  const c = data?.crossfade;
  if (!on('crossfade') || !c?.duration_ms || ad || repeatOne(v)) return null;
  const dur = c.duration_ms / 1000;
  const end = v.duration;
  if (!Number.isFinite(end) || end < 3 * dur) return null;
  return { out: end - dur, dur, curve: CURVES[c.curve] || CURVES.equal_power, echo: !!c.echo };
};

// Schedules the fade-out (or what is left of it) from the video's time now.
const arm = (v) => {
  if (!fade || ad) return;
  const g = nodes.master.gain;
  const now = ctx.currentTime;
  const rate = v.playbackRate || 1;
  const begin = Math.max(now + RAMP, now + (fade.out - v.currentTime) / rate);
  const p = Math.min(1, Math.max(0, (v.currentTime + (begin - now) * rate - fade.out) / fade.dur));
  g.cancelAndHoldAtTime(now);
  g.linearRampToValueAtTime(valueAt(fade.curve, p), begin);
  const length = ((1 - p) * fade.dur) / rate;
  try {
    if (length > 0.02) g.setValueCurveAtTime(slice(fade.curve, p), begin + 0.001, length);
  } catch {
    g.linearRampToValueAtTime(0, begin + length);
  }
  armed = true;
  if (fade.echo) armEcho(v, begin, p, length);
};

// The echo: the send opens for the middle of the fade, so its repeats ring on after the track.
const armEcho = (v, begin, p, length) => {
  const beat = facts()?.bpm ? 60 / facts().bpm : ECHO_DEFAULT_S;
  nodes.delay.delayTime.setValueAtTime(Math.min(1.5, beat), ctx.currentTime);
  const send = nodes.echoSend.gain;
  send.cancelAndHoldAtTime(ctx.currentTime);
  const at = (q) => begin + Math.max(0, ((q - p) / (1 - p || 1)) * length);
  send.setValueAtTime(0, ctx.currentTime);
  if (p < 0.85) {
    send.setTargetAtTime(1, at(0.5), RAMP / 3);
    send.setTargetAtTime(0, at(0.85), RAMP / 3);
  }
  ramp(nodes.echoWet.gain, ECHO_WET);
  echo.on = true;
  echo.until = begin + length + ECHO_RING_S;
  wire();
};
const stopEcho = (now) => {
  if (!echo.on) return;
  ramp(nodes.echoSend.gain, 0);
  if (now) ramp(nodes.echoWet.gain, 0);
};

// Pause, seek, rate change or ad: hold the fade where it is; it is armed again from timeupdate.
const holdFade = () => {
  if (!armed) return;
  nodes.master.gain.cancelAndHoldAtTime(ctx.currentTime);
  nodes.echoSend.gain.cancelAndHoldAtTime(ctx.currentTime);
  armed = false;
};
// Back to full level in 30 ms, nothing scheduled.
const cancelFade = () => {
  armed = false;
  ramp(nodes.master.gain, 1);
  stopEcho(false);
};

const inWindow = (t) => fade && t >= fade.out - ARM_BEFORE_S && t < fade.out + fade.dur;

// The next track starts: it fades in if the last one faded out (a short fade if it was cut off).
const startTrack = (v, id) => {
  const cut = armed && !faded;
  const prev = nodes.master.gain.value;
  trackId = id;
  armed = false;
  faded = false;
  fade = planFade(v);
  if (ad || !on('crossfade') || prev > 0.98) return;
  const curveName = data?.crossfade?.curve;
  const dur = cut ? SKIP_FADE_IN_S : (data?.crossfade?.fade_in_ms ?? 0) / 1000;
  const g = nodes.master.gain;
  const now = ctx.currentTime;
  g.cancelAndHoldAtTime(now);
  if (dur < 0.05) {
    g.setTargetAtTime(1, now, RAMP / 3);
    return;
  }
  // From where it is now to 1, along the fade-in curve.
  const curve = FADE_IN[curveName] || FADE_IN.equal_power;
  const scaled = Float32Array.from(curve, (x) => prev + (1 - prev) * x);
  try {
    g.setValueCurveAtTime(scaled, now + 0.001, dur);
  } catch {
    g.linearRampToValueAtTime(1, now + dur);
  }
};

const onTime = (v) => {
  if (echo.on && ctx.currentTime > echo.until) {
    echo.on = false;
    wire();
  }
  if (ad || v.paused) return;
  const id = currentId();
  if (id && id !== trackId) startTrack(v, id);
  if (!fade && on('crossfade')) fade = planFade(v);
  const t = v.currentTime;
  // Repeat of the same track after its fade (a one-track queue on repeat-all): back in.
  if (faded && t < 2) startTrack(v, id);
  if (!fade) return;
  if (!armed && inWindow(t)) arm(v);
  // The fade is over: let the track end by itself (unless the fade already ends with it).
  if (armed && !faded && t >= fade.out + fade.dur - 0.05) {
    faded = true;
    if (fade.out + fade.dur < v.duration - 0.1) seekTo(v, v.duration - 0.05);
  }
};

const onSeek = (v, type) => {
  if (ownSeek) {
    if (type === 'seeked') ownSeek = false;
    return;
  }
  if (type !== 'seeking') return;
  holdFade();
  if (!inWindow(v.currentTime)) {
    cancelFade();
    faded = false;
  }
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
// An ad: nothing stays scheduled (apply() brings every parameter back to neutral).
const onAd = (now) => {
  if (!now || !nodes) return;
  armed = false;
  fade = null;
  nodes.master.gain.cancelAndHoldAtTime(ctx.currentTime);
  stopEcho(true);
};

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
      holdFade();
      clearTimeout(suspendTimer);
      suspendTimer = setTimeout(() => video?.paused && ctx.suspend().catch(() => {}), SUSPEND_MS);
      break;
    case 'ratechange':
      holdFade();
      break;
    case 'seeking':
    case 'seeked':
      onSeek(v, e.type);
      break;
    case 'loadstart':
      ownSeek = false;
      break;
    case 'loadedmetadata':
    case 'durationchange':
      // A new duration: the fade is planned again.
      if (!armed) fade = null;
      break;
    case 'playing':
    case 'timeupdate':
      if (e.type === 'timeupdate') checkGuard(v);
      onTime(v);
      break;
  }
};
for (const type of ['play', 'pause', 'playing', 'timeupdate', 'loadstart', 'loadedmetadata', 'durationchange', 'seeking', 'seeked', 'ratechange']) {
  document.addEventListener(type, onMedia, true);
}

feathertune.audio = Object.freeze({
  set: (d) => {
    data = d;
    if (off) return;
    // New crossfade settings: planned again (a fade under way is cut if the plugin went off).
    if (nodes && armed && !on('crossfade')) cancelFade();
    if (!armed) fade = null;
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
