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
  if (tapNode) list.push([nodes.input, tapNode], [tapNode, nodes.zero], [nodes.zero, ctx.destination]);
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
  // The module is the analysis tap itself (see track analysis).
  const url = URL.createObjectURL(new Blob([TAP_CODE], { type: 'application/javascript' }));
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
const facts = () => {
  const f = data?.facts?.current;
  return f && f.id === trackId ? f : null;
};
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
// `silence`: the time skipped is known to be silent (skip_silence); otherwise the analysis of the
// track ends where it got to.
const seekTo = (v, t, silence = false) => {
  if (silence) skipAnalysis(t);
  else if (analysis) stopAnalysis(true);
  ownSeek = true;
  v.currentTime = t;
};

// The fade for this track from the global settings (a mix replaces it, see below).
const planFade = (v) => {
  const c = data?.crossfade;
  if (!on('crossfade') || !c?.duration_ms || ad || repeatOne(v)) return null;
  const dur = c.duration_ms / 1000;
  // Before the final silence, once it is known.
  const tail = facts()?.tail_ms;
  const end = tail && tail / 1000 < v.duration ? tail / 1000 : v.duration;
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
  if (analysis && analysis.id !== id) stopAnalysis(analysis.t >= analysis.duration - 1.5);
  const cut = armed && !faded;
  const prev = nodes.master.gain.value;
  trackId = id;
  armed = false;
  faded = false;
  resetQuiet();
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
  if (analysis && Math.abs(analysis.t - t) > 0.5) {
    // Out of step (buffering): the envelope goes on from here, the beat window is lost.
    if (inBeatWindow(analysis)) analysis.beatOk = false;
    analysis.t = t;
  }
  if (id) startAnalysis(v, id);
  skipSilence(v, t);
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
  // A seek by the user: this track is not measured in this session.
  stopAnalysis(false);
  quiet.userSeek = true;
  holdFade();
  if (!inWindow(v.currentTime)) {
    cancelFade();
    faded = false;
  }
};

// --- track analysis ---
//
// The first time a track plays from its start (normal play: no ad, rate 1, no seek by the user),
// a tap on the input reads the samples and works out, on this page only:
// - an envelope: the level of three bands (< 250 Hz, mid, > 4 kHz) every 100 ms, as bytes;
// - where the sound starts and ends (above -50 dBFS);
// - the tempo and first beat: onset strength (spectral flux) over the first 60 s of sound,
//   autocorrelated between 60 and 200 BPM;
// - the key: a chromagram over the same 60 s against the Krumhansl profiles (an estimate).
// Only these numbers leave the page (audio_facts); the samples are never kept. The tap is an
// AudioWorklet when the site's CSP lets a Blob module load, else a ScriptProcessor; it outputs
// silence into a zero gain, and exists only while a track is being measured.

const A = {
  FRAME: 1024,
  HOP: 512,
  CHROMA: 8192,
  CHROMA_EVERY_S: 0.5,
  BEAT_S: 60,
  BIN_S: 0.1,
  MAX_BINS: 9000,
  SILENCE_DB: -50,
  MIN_BEAT_S: 20,
};
const KEY_NAMES = ['C', 'C#', 'D', 'Eb', 'E', 'F', 'F#', 'G', 'Ab', 'A', 'Bb', 'B'];
const MAJOR = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR = [6.33, 2.68, 3.52, 5.38, 2.6, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const TAP_CODE = `registerProcessor('feathertune-tap', class extends AudioWorkletProcessor {
  constructor() { super(); this.buf = new Float32Array(4096); this.n = 0; }
  process(inputs) {
    const ch = inputs[0];
    if (!ch || !ch.length) return true;
    const a = ch[0], b = ch[1] || ch[0];
    for (let i = 0; i < a.length; i++) {
      this.buf[this.n++] = (a[i] + b[i]) / 2;
      if (this.n === this.buf.length) { this.port.postMessage(this.buf, [this.buf.buffer]); this.buf = new Float32Array(4096); this.n = 0; }
    }
    return true;
  }
});`;

// In-place radix-2 FFT (re, im of length n, a power of two), with tables kept per size.
const fftTables = new Map();
const fft = (re, im) => {
  const n = re.length;
  let t = fftTables.get(n);
  if (!t) {
    const rev = new Uint32Array(n);
    for (let i = 0, j = 0; i < n; i++) {
      rev[i] = j;
      let bit = n >> 1;
      for (; j & bit; bit >>= 1) j ^= bit;
      j |= bit;
    }
    const cos = new Float32Array(n / 2);
    const sin = new Float32Array(n / 2);
    for (let i = 0; i < n / 2; i++) {
      cos[i] = Math.cos((2 * Math.PI * i) / n);
      sin[i] = -Math.sin((2 * Math.PI * i) / n);
    }
    const win = Float32Array.from({ length: n }, (_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / n));
    t = { rev, cos, sin, win };
    fftTables.set(n, t);
  }
  for (let i = 0; i < n; i++) {
    const j = t.rev[i];
    if (j > i) {
      [re[i], re[j]] = [re[j], re[i]];
      [im[i], im[j]] = [im[j], im[i]];
    }
  }
  for (let size = 2; size <= n; size <<= 1) {
    const half = size >> 1;
    const step = n / size;
    for (let start = 0; start < n; start += size) {
      for (let k = 0; k < half; k++) {
        const wr = t.cos[k * step];
        const wi = t.sin[k * step];
        const a = start + k;
        const b = a + half;
        const xr = re[b] * wr - im[b] * wi;
        const xi = re[b] * wi + im[b] * wr;
        re[b] = re[a] - xr;
        im[b] = im[a] - xi;
        re[a] += xr;
        im[a] += xi;
      }
    }
  }
  return t;
};
// The windowed spectrum's magnitudes of the last n samples of `ring` (a circular buffer at `pos`).
const spectrum = (ring, pos, n) => {
  const re = new Float32Array(n);
  const im = new Float32Array(n);
  const len = ring.length;
  for (let i = 0; i < n; i++) re[i] = ring[(pos - n + i + len) % len];
  const t = fftTables.get(n) || fft(new Float32Array(n), new Float32Array(n));
  for (let i = 0; i < n; i++) re[i] *= t.win[i];
  fft(re, im);
  const mag = new Float32Array(n / 2);
  for (let k = 0; k < n / 2; k++) mag[k] = Math.hypot(re[k], im[k]);
  return mag;
};

let tapNode = null;
let tapKind = null;
let analysis = null;
// Tracks measured or given up in this page session.
const analysed = new Set();

const ensureTap = () => {
  if (tapNode) return;
  nodes.zero ||= ctx.createGain();
  nodes.zero.gain.value = 0;
  if (worklet === 'ok') {
    tapNode = new AudioWorkletNode(ctx, 'feathertune-tap', { numberOfOutputs: 1, outputChannelCount: [1] });
    tapNode.port.onmessage = (e) => feed(e.data);
    tapKind = 'worklet';
  } else {
    tapNode = ctx.createScriptProcessor(4096, 2, 1);
    tapNode.onaudioprocess = (e) => {
      const input = e.inputBuffer;
      const a = input.getChannelData(0);
      const b = input.numberOfChannels > 1 ? input.getChannelData(1) : a;
      const mono = new Float32Array(a.length);
      for (let i = 0; i < a.length; i++) mono[i] = (a[i] + b[i]) / 2;
      feed(mono);
    };
    tapKind = 'script';
  }
};
const dropTap = () => {
  if (!tapNode) return;
  const node = tapNode;
  tapNode = null;
  wire();
  if (node.port) node.port.onmessage = null;
  else node.onaudioprocess = null;
};

const startAnalysis = (v, id) => {
  if (!on('track_analysis') || analysed.has(id) || analysis || ad || v.playbackRate !== 1) return;
  if (facts()?.id === id || v.currentTime > 0.5 || !Number.isFinite(v.duration)) return;
  const sr = ctx.sampleRate;
  analysis = {
    id,
    sr,
    duration: v.duration,
    t: v.currentTime,
    ring: new Float32Array(A.CHROMA),
    pos: 0,
    sinceHop: 0,
    sinceChroma: 0,
    prev: null,
    // Per 100 ms bin: summed power of low, mid, high and the frame count.
    bins: new Float32Array(Math.min(A.MAX_BINS, Math.ceil(v.duration / A.BIN_S) + 1) * 4),
    covered: 0,
    soundAt: null,
    onsets: [],
    onsetFrom: null,
    beatOk: true,
    chroma: new Float32Array(12),
    chromaFrames: 0,
  };
  ensureTap();
  wire();
};

const stopAnalysis = (send) => {
  if (!analysis) return;
  const a = analysis;
  analysis = null;
  analysed.add(a.id);
  dropTap();
  if (send) {
    const facts = finish(a);
    if (facts) window.__TAURI_INTERNALS__?.invoke('audio_facts', { facts: JSON.stringify(facts) }).catch(() => {});
  }
};

// The samples of one block (mono). Dropped while paused, buffering, during an ad or after a seek.
const feed = (samples) => {
  const a = analysis;
  const v = video;
  if (!a || !v || v.paused || v.readyState < 3 || ad) return;
  const nyquist = a.sr / 2;
  const t0 = a.t;
  for (let i = 0; i < samples.length; i++) {
    a.t = t0 + i / a.sr;
    a.ring[a.pos] = samples[i];
    a.pos = (a.pos + 1) % a.ring.length;
    if (++a.sinceHop >= A.HOP) {
      a.sinceHop = 0;
      frame(a, nyquist);
    }
    if (++a.sinceChroma >= A.CHROMA_EVERY_S * a.sr) {
      a.sinceChroma = 0;
      if (inBeatWindow(a)) chromaFrame(a);
    }
  }
  a.t = t0 + samples.length / a.sr;
};
const inBeatWindow = (a) => a.onsetFrom !== null && a.t - a.onsetFrom < A.BEAT_S;

// One 1024-sample frame every 512 samples: band levels for the envelope and the onset strength.
const frame = (a, nyquist) => {
  const mag = spectrum(a.ring, a.pos, A.FRAME);
  const n = A.FRAME;
  // Mean square per band (Parseval with the Hann window: sum of w^2 is 3n/8).
  const norm = 2 / (n * ((3 * n) / 8));
  let low = 0;
  let mid = 0;
  let high = 0;
  for (let k = 1; k < mag.length; k++) {
    const f = (k * nyquist) / mag.length;
    const p = mag[k] * mag[k] * norm;
    if (f < 250) low += p;
    else if (f < 4000) mid += p;
    else high += p;
  }
  const bin = Math.floor(a.t / A.BIN_S);
  if (bin < a.bins.length / 4) {
    a.bins[bin * 4] += low;
    a.bins[bin * 4 + 1] += mid;
    a.bins[bin * 4 + 2] += high;
    a.bins[bin * 4 + 3] += 1;
    a.covered = Math.max(a.covered, bin + 1);
  }
  const db = 10 * Math.log10(low + mid + high + 1e-12) + 3.01;
  if (db > A.SILENCE_DB && a.soundAt === null) {
    a.soundAt = a.t;
    a.onsetFrom = a.t;
  }
  if (inBeatWindow(a) && a.beatOk) {
    let flux = 0;
    if (a.prev) for (let k = 1; k < mag.length; k++) flux += Math.max(0, Math.log1p(100 * mag[k]) - Math.log1p(100 * a.prev[k]));
    a.onsets.push(flux);
  }
  a.prev = mag;
};

const chromaFrame = (a) => {
  const mag = spectrum(a.ring, a.pos, A.CHROMA);
  const df = a.sr / A.CHROMA;
  for (let k = Math.ceil(65 / df); k < Math.min(mag.length, 2000 / df); k++) {
    const midi = 69 + 12 * Math.log2((k * df) / 440);
    a.chroma[((Math.round(midi) % 12) + 12) % 12] += mag[k];
  }
  a.chromaFrames++;
};

// Our own forward seek over silence (skip_silence): the skipped time counts as silent.
const skipAnalysis = (to) => {
  if (analysis && to > analysis.t) analysis.t = to;
};

const finish = (a) => {
  const bins = a.covered;
  if (bins < 10) return null;
  const bytes = new Uint8Array(bins * 3);
  const toByte = (p, frames) => {
    const db = frames ? 10 * Math.log10(p / frames + 1e-12) + 3.01 : -60;
    return Math.round(((Math.min(0, Math.max(-60, db)) + 60) / 60) * 255);
  };
  let first = null;
  let last = null;
  for (let b = 0; b < bins; b++) {
    const frames = a.bins[b * 4 + 3];
    for (let band = 0; band < 3; band++) bytes[b * 3 + band] = toByte(a.bins[b * 4 + band], frames);
    const sum = a.bins[b * 4] + a.bins[b * 4 + 1] + a.bins[b * 4 + 2];
    const db = frames ? 10 * Math.log10(sum / frames + 1e-12) + 3.01 : -Infinity;
    if (db > A.SILENCE_DB) {
      first ??= b;
      last = b;
    }
  }
  let b64 = '';
  for (let i = 0; i < bytes.length; i += 3000) b64 += String.fromCharCode(...bytes.subarray(i, i + 3000));
  const ms = (x) => Math.round(x * 1000);
  const durationMs = Math.floor(a.duration * 1000);
  const facts = {
    id: a.id,
    duration_s: Math.round(a.duration * 1000) / 1000,
    lead_in_ms: Math.min(durationMs, ms((first ?? 0) * A.BIN_S)),
    envelope: btoa(b64),
  };
  // The end is known only if it was heard.
  if (last !== null && a.t >= a.duration - 0.5) facts.tail_ms = Math.max(facts.lead_in_ms, Math.min(durationMs, ms((last + 1) * A.BIN_S)));
  const fps = a.sr / A.HOP;
  if (a.beatOk && a.onsets.length / fps >= A.MIN_BEAT_S) Object.assign(facts, tempo(a.onsets, fps, a.onsetFrom));
  if (a.chromaFrames >= 20) Object.assign(facts, key(a.chroma));
  return facts;
};

// Tempo by autocorrelation of the onset strength, weighted toward 120 BPM (one octave either way).
const tempo = (onsets, fps, from) => {
  const mean = onsets.reduce((s, x) => s + x, 0) / onsets.length;
  const o = onsets.map((x) => Math.max(0, x - mean));
  const lo = Math.floor((fps * 60) / 200);
  const hi = Math.ceil((fps * 60) / 60);
  const r = new Float64Array(hi + 2);
  for (let lag = lo - 1; lag <= hi + 1; lag++) {
    let s = 0;
    for (let i = 0; i + lag < o.length; i++) s += o[i] * o[i + lag];
    r[lag] = s;
  }
  let best = -1;
  let bestScore = -Infinity;
  for (let lag = lo; lag <= hi; lag++) {
    const bpm = (60 * fps) / lag;
    const score = r[lag] * Math.exp(-0.5 * Math.pow(Math.log2(bpm / 120), 2));
    if (score > bestScore && r[lag] >= r[lag - 1] && r[lag] >= r[lag + 1]) {
      bestScore = score;
      best = lag;
    }
  }
  if (best < 0 || !(bestScore > 0)) return {};
  const [a, b, c] = [r[best - 1], r[best], r[best + 1]];
  const shift = a - 2 * b + c ? (0.5 * (a - c)) / (a - 2 * b + c) : 0;
  const period = best + Math.max(-0.5, Math.min(0.5, shift));
  const bpm = Math.round(((60 * fps) / period) * 10) / 10;
  // The phase whose beats land on the most onset strength.
  let phase = 0;
  let phaseScore = -1;
  for (let p = 0; p < period; p++) {
    let s = 0;
    for (let i = p; i < o.length; i += period) s += o[Math.round(i)] || 0;
    if (s > phaseScore) {
      phaseScore = s;
      phase = p;
    }
  }
  const beatS = 60 / bpm;
  const first = from + phase / fps;
  return { bpm, beat_offset_ms: Math.round((first - Math.floor(first / beatS) * beatS) * 1000) };
};

// The key whose profile correlates best with the chroma; also on the Camelot wheel.
const key = (chroma) => {
  const corr = (x, y) => {
    const mx = x.reduce((s, v) => s + v, 0) / 12;
    const my = y.reduce((s, v) => s + v, 0) / 12;
    let n = 0;
    let dx = 0;
    let dy = 0;
    for (let i = 0; i < 12; i++) {
      n += (x[i] - mx) * (y[i] - my);
      dx += (x[i] - mx) ** 2;
      dy += (y[i] - my) ** 2;
    }
    return dx && dy ? n / Math.sqrt(dx * dy) : -1;
  };
  let best = null;
  for (let tonic = 0; tonic < 12; tonic++) {
    for (const [mode, profile] of [['major', MAJOR], ['minor', MINOR]]) {
      const rotated = profile.map((_, i) => profile[(i - tonic + 12) % 12]);
      const c = corr(chroma, rotated);
      if (!best || c > best.c) best = { tonic, mode, c };
    }
  }
  if (!best || best.c <= 0) return {};
  // Camelot: C major is 8B and each fifth up adds one; a minor key sits with its relative major.
  const major = best.mode === 'major' ? best.tonic : (best.tonic + 3) % 12;
  const number = ((((major * 7) % 12) + 7) % 12) + 1;
  return { key: `${KEY_NAMES[best.tonic]} ${best.mode}`, camelot: `${number}${best.mode === 'major' ? 'B' : 'A'}` };
};

// --- skip silence (start and end only, never in the middle) ---
//
// Start: with the analysis' lead-in (over 1.5 s), one seek to it; without it, the input level is read
// on each timeupdate (about 4 a second) for the first 15 s, and after 1.5 s under -50 dBFS it seeks
// 2 s on, at most 4 times, stopping at the first sound. End: with the known end of the sound, the
// crossfade already stops there; without a crossfade it seeks to the end once the sound is over (or
// after 1.5 s of silence in the last 20 s, when the end is not known). Never during an ad, and never
// on a track the user has seeked in.

const QUIET = { MIN_LEAD_S: 1.5, FOR_S: 1.5, STEP_S: 2, MAX_STEPS: 4, START_S: 15, END_S: 20 };
let quiet = { userSeek: false, startDone: false, endDone: false, steps: 0, from: null };

const resetQuiet = () => {
  quiet = { userSeek: false, startDone: false, endDone: false, steps: 0, from: null };
  useAnalyser('silence', false);
};
// Whether the input has been under the threshold for long enough (reading the analyser now).
const quietFor = (t) => {
  useAnalyser('silence', true);
  if (level() > A.SILENCE_DB) {
    quiet.from = null;
    return false;
  }
  quiet.from ??= t;
  return t - quiet.from >= QUIET.FOR_S;
};
const skipSilence = (v, t) => {
  if (!on('skip_silence') || quiet.userSeek || ad) return useAnalyser('silence', false);
  const f = facts();
  const end = v.duration;
  let listening = false;
  if (!quiet.startDone) {
    if (f) {
      quiet.startDone = true;
      const lead = f.lead_in_ms / 1000;
      if (lead > QUIET.MIN_LEAD_S && t < lead - 0.5) seekTo(v, lead - 0.1, true);
    } else if (t > QUIET.START_S || quiet.steps >= QUIET.MAX_STEPS) {
      quiet.startDone = true;
    } else {
      listening = true;
      if (level() > A.SILENCE_DB && analyserUsers.has('silence')) quiet.startDone = true;
      else if (quietFor(t)) {
        quiet.steps++;
        quiet.from = null;
        seekTo(v, t + QUIET.STEP_S, true);
      }
    }
  } else if (!fade && !quiet.endDone && Number.isFinite(end)) {
    if (f?.tail_ms) {
      const tail = f.tail_ms / 1000;
      if (tail < end - 1 && t >= tail) {
        quiet.endDone = true;
        seekTo(v, end - 0.05, true);
      }
    } else if (t >= end - QUIET.END_S && t < end - 1) {
      listening = true;
      if (quietFor(t)) {
        quiet.endDone = true;
        seekTo(v, end - 0.05, true);
      }
    }
  }
  if (!listening) useAnalyser('silence', false);
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
  stopAnalysis(false);
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
      if (v.playbackRate !== 1) stopAnalysis(false);
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
    if (analysis && d?.facts?.current?.id === analysis.id) stopAnalysis(false);
      break;
    case 'playing':
      // After a pause or a stall, the samples dropped meanwhile: back in step.
      if (analysis) analysis.t = v.currentTime;
      onTime(v);
      break;
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
