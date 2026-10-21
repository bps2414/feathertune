// The transition voice (deck.rs): the only script of the hidden `deck` webview, which has no
// capability (it can call nothing). It loads the next track, holds every play() until Rust says
// `go`, and plays only the start of the track during a mix's overlap, through its own small graph
// (gain and a low shelf for the bass swap). Rust reads `poll()` with ExecuteScript and treats what
// it returns as untrusted data.
// - An ad is never played here, not even muted or hidden: while the player shows one, nothing
//   plays and `poll()` says so (Rust then closes this webview and falls back to one player).
// - Every gain change is scheduled on the audio clock, at a wall-clock time Rust gives (the same
//   clock as the main page's: performance.timeOrigin + performance.now()).
if (window.top !== window) return;

const SELECTORS = {
  player: '#movie_player',
  // The site's playback error (an account in use elsewhere shows here too).
  error: 'yt-playability-error-supported-renderers, .ytp-error',
};
const CURVE_POINTS = 256;
const RAMP = 0.03;
// The bass swap: the low shelf of the incoming track, held down until halfway.
const BASS_HZ = 200;
const BASS_CUT_DB = -24;

const wall = () => performance.timeOrigin + performance.now();
const realPlay = HTMLMediaElement.prototype.play;
let allowed = false;
let target = null;
let seeked = false;
let started = false;
let stopped = false;
let ctx = null;
let gain = null;
let bass = null;

// Nothing plays until `go`: play() waits (the site's promise stays pending), autoplay is paused.
HTMLMediaElement.prototype.play = function () {
  if (allowed) return realPlay.call(this);
  return new Promise(() => {});
};
document.addEventListener(
  'play',
  (e) => {
    if (!allowed && e.target instanceof HTMLMediaElement) e.target.pause();
  },
  true,
);

const video = () => document.querySelector('video');
const adNow = () => {
  const p = document.querySelector(SELECTORS.player);
  return !p || p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting');
};

// The fade-in curve (0 to 1), the main page's fade-out backwards.
const fadeIn = (name) => {
  const f = {
    linear: (t) => t,
    s_curve: (t) => 0.5 * (1 - Math.cos(Math.PI * t)),
    exponential: (t) => (Math.pow(10, -3 * (1 - t)) - 0.001) / 0.999,
  }[name] || ((t) => Math.sin((t * Math.PI) / 2));
  return Float32Array.from({ length: CURVE_POINTS }, (_, i) => f(i / (CURVE_POINTS - 1)));
};
// A wall-clock time on this context's clock.
const at = (wallMs) => {
  const ts = ctx.getOutputTimestamp?.();
  if (ts && ts.performanceTime > 0) return ts.contextTime + (wallMs - performance.timeOrigin - ts.performanceTime) / 1000;
  return ctx.currentTime + (wallMs - wall()) / 1000;
};

const stop = () => {
  stopped = true;
  allowed = false;
  const v = video();
  if (gain) {
    gain.gain.cancelAndHoldAtTime(ctx.currentTime);
    gain.gain.setTargetAtTime(0, ctx.currentTime, RAMP / 3);
  }
  setTimeout(() => v?.pause(), 60);
};

window.__feathertuneDeck = Object.freeze({
  // The state, and (the first time) where to wait: `in_s`, the incoming track's start.
  poll: (inS) => {
    const v = video();
    const ad = adNow();
    if (ad && started && !stopped) stop();
    if (v && target === null && Number.isFinite(inS) && v.readyState >= 1 && !ad) {
      target = Math.max(0, inS);
      v.addEventListener('seeked', () => (seeked = true), { once: true });
      if (target > 0.05) v.currentTime = target;
      else seeked = true;
    }
    const data = v ? document.querySelector(SELECTORS.player)?.getVideoData?.() : null;
    return {
      id: data?.video_id ?? null,
      ad,
      ready: !!v && seeked && v.readyState >= 3 && !ad,
      started,
      paused: v ? v.paused : true,
      t: v ? v.currentTime : 0,
      rate: v ? v.playbackRate : 1,
      wall: wall(),
      error: !!(v?.error || document.querySelector(SELECTORS.error)?.offsetParent),
    };
  },
  // Plays now, silent until `at_ms`, then fades in along the curve over `dur_ms`. `rate`: the tempo
  // effect (preserving pitch). `bass_swap`: the lows held down until halfway, swapped in `beat_ms`.
  go: (o) => {
    const v = video();
    if (!v || started || adNow()) return false;
    ctx = new AudioContext({ latencyHint: 'playback' });
    const source = ctx.createMediaElementSource(v);
    gain = ctx.createGain();
    gain.gain.value = 0;
    bass = ctx.createBiquadFilter();
    bass.type = 'lowshelf';
    bass.frequency.value = BASS_HZ;
    bass.gain.value = o.bass_swap ? BASS_CUT_DB : 0;
    source.connect(bass).connect(gain).connect(ctx.destination);
    const start = at(o.at_ms);
    const dur = Math.max(0.05, o.dur_ms / 1000);
    gain.gain.setValueAtTime(0, ctx.currentTime);
    try {
      gain.gain.setValueCurveAtTime(fadeIn(o.curve), Math.max(ctx.currentTime + 0.01, start), dur);
    } catch {
      gain.gain.linearRampToValueAtTime(1, start + dur);
    }
    if (o.bass_swap) {
      const half = start + dur / 2;
      bass.gain.setValueAtTime(BASS_CUT_DB, half);
      bass.gain.linearRampToValueAtTime(0, half + Math.max(0.05, o.beat_ms / 1000));
    }
    if (o.rate > 0.9 && o.rate < 1.1) {
      v.preservesPitch = true;
      v.playbackRate = o.rate;
    }
    allowed = true;
    started = true;
    realPlay.call(v).catch(() => {});
    return true;
  },
  // The main page took over: down to silence over `ms` from `at_ms`, then paused.
  fadeOut: (o) => {
    if (!gain) return stop();
    const g = gain.gain;
    const from = Math.max(ctx.currentTime, at(o.at_ms));
    g.cancelAndHoldAtTime(from);
    g.linearRampToValueAtTime(0, from + Math.max(0.03, o.ms / 1000));
  },
  stop,
});
