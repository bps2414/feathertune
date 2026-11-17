// The transition voice (deck.rs): the only script of the hidden `deck` webview, which has no
// capability (it can call nothing). It loads the track the main page plays now, waits a few seconds
// before that page's fade (`poll(target)`), holds every play() until Rust says `go`, then plays along
// silently while Rust keeps it in step with the main page (`sync`). At the swap the two cross in 60 ms
// (the same audio, so the level holds) and from then on only this page plays the end of the track,
// fading out along the curve with the transition's effects (echo, rising filter, bass swap), while the
// main page has already moved on to the next track. Its graph copies the main page's equalizer and
// compressor (`chain`), so the level does not jump at the swap, and a gain matches the main page's
// volume. Rust reads `poll()` / `sync()` with ExecuteScript and treats what they return as untrusted data.
// - An ad is never played here, not even muted or hidden: while the player shows one, nothing
//   plays and the state says so (Rust then closes this webview). While the page still has no player
//   nothing plays either, and the state says `loading`, not `ad`.
// - When its track ends (or the site moves this page to another track) it stops: nothing else plays.
// - Every gain change is scheduled on the audio clock, at a time on this page's wall clock
//   (performance.timeOrigin + performance.now()), which Rust has already moved from the main page's
//   (each process anchors its wall clock at its own start; Rust measures the difference).
if (window.top !== window) return;

const SELECTORS = {
  player: '#movie_player',
  // The site's playback error (an account in use elsewhere shows here too).
  error: 'yt-playability-error-supported-renderers, .ytp-error',
};
// The track this page was opened for.
const EXPECT = new URLSearchParams(location.search).get('v');
const CURVE_POINTS = 256;
const RAMP = 0.03;
// The equalizer's bands (as in the audio module of the main page).
const EQ_FREQUENCIES = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_Q = 1.4;
// The bass swap: the low shelf of the outgoing track goes down at the fade's half.
const BASS_HZ = 200;
const BASS_CUT_DB = -24;
// The filter: a high-pass rising over the fade.
const SWEEP_FROM_HZ = 20;
const SWEEP_TO_HZ = 1000;
// The echo: open from 50% to 85% of the fade, a beat long (at most 1.5 s).
const ECHO_FEEDBACK = 0.45;
const ECHO_WET = 0.5;
const MAX_LEVEL = 4;
// The lock: past this, one seek; within it, the rate corrects by twice the offset (at most 8%).
const SEEK_OVER_S = 0.3;
const SEEK_AHEAD_S = 0.2;
const RATE_GAIN = 2;
const MAX_CORRECTION = 0.08;
const LOCK_S = 0.012;
const LOCK_SYNCS = 3;
// It stops correcting this long before the swap.
const FREEZE_S = 0.2;

const wall = () => performance.timeOrigin + performance.now();
const realPlay = HTMLMediaElement.prototype.play;
let allowed = false;
let target = null;
let seeked = false;
let started = false;
let ended = false;
// Waiting for its moment to play, after `go`.
let pending = false;
let blocked = null;
let stopped = false;
let ctx = null;
let n = null;
// The lock: a seek of ours under way, the last syncs within LOCK_S, the main page's rate, frozen.
let correcting = false;
let recent = [];
let mainRate = 1;
let frozen = false;
let offsetMs = null;

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
const player = () => document.querySelector(SELECTORS.player);
// An ad on the player now (what Rust acts on).
const adShowing = () => {
  const p = player();
  return !!p && (p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting'));
};
// Nothing plays without a player either (it may be about to show an ad), but that is not an ad yet.
const adNow = () => !player() || adShowing();
// What is wrong with playback, or null. The site keeps its error element in the page, empty and
// rendered, while there is no error: only one with something to say counts.
const errorOf = (v) => {
  if (v?.error) return `media error ${v.error.code}`;
  const e = document.querySelector(SELECTORS.error);
  return (e?.offsetParent && e.textContent.trim().slice(0, 200)) || null;
};
const num = (x, lo, hi, fallback) => (Number.isFinite(x) ? Math.min(hi, Math.max(lo, x)) : fallback);

// Fade-out curves, from 1 to 0 (the main page fades its next track in along the same curve backwards).
const fadeOut = (name) => {
  const f =
    {
      linear: (t) => 1 - t,
      s_curve: (t) => 0.5 * (1 + Math.cos(Math.PI * t)),
      exponential: (t) => (Math.pow(10, -3 * t) - 0.001) / 0.999,
    }[name] || ((t) => Math.cos((t * Math.PI) / 2));
  return Float32Array.from({ length: CURVE_POINTS }, (_, i) => f(i / (CURVE_POINTS - 1)));
};
// A wall-clock time (this page's) on this context's clock.
const at = (wallMs) => {
  const ts = ctx.getOutputTimestamp?.();
  if (ts && ts.performanceTime > 0) return ts.contextTime + (wallMs - performance.timeOrigin - ts.performanceTime) / 1000;
  return ctx.currentTime + (wallMs - wall()) / 1000;
};
// Calls `fn` when the audio clock reaches `time` (this page is hidden, and its timers are held back;
// the timer is there in case the clock does not run).
const onClock = (time, fn) => {
  let done = false;
  const once = () => {
    if (done) return;
    done = true;
    fn();
  };
  const clock = ctx.createConstantSource();
  const silent = ctx.createGain();
  silent.gain.value = 0;
  clock.connect(silent).connect(ctx.destination);
  clock.onended = () => {
    silent.disconnect();
    once();
  };
  clock.start();
  clock.stop(Math.max(ctx.currentTime, time));
  setTimeout(once, (Math.max(0, time - ctx.currentTime) + 0.25) * 1000);
};

// Silence at once (an ad, a fallback): everything, the echo too, then paused.
const stop = () => {
  stopped = true;
  allowed = false;
  const v = video();
  if (n) {
    n.level.gain.cancelAndHoldAtTime(ctx.currentTime);
    n.level.gain.setTargetAtTime(0, ctx.currentTime, RAMP / 3);
  }
  setTimeout(() => v?.pause(), 60);
};

// The track ended (or the site moved this page on): nothing else plays; the echo may ring on.
document.addEventListener(
  'ended',
  (e) => {
    if (!started || e.target !== video()) return;
    ended = true;
    allowed = false;
  },
  true,
);
document.addEventListener(
  'seeked',
  (e) => {
    if (e.target !== video()) return;
    seeked = true;
    correcting = false;
  },
  true,
);

// Plays, unless it was stopped or an ad showed up meanwhile.
const begin = () => {
  if (allowed || stopped || ended) return;
  if (adNow()) return stop();
  pending = false;
  allowed = true;
  realPlay.call(video()).catch((e) => (blocked = String(e?.name)));
};
// The gain that makes this <video> as loud as the main page's (its own volume is the site's to set).
const levelOf = (l, v) => (l?.muted || !v.volume ? 0 : Math.min(MAX_LEVEL, num(l?.volume, 0, 1, 0) / v.volume));

const state = () => {
  const v = video();
  const held = adNow();
  if (held && started && !stopped) stop();
  const data = v ? player()?.getVideoData?.() : null;
  const id = data?.video_id ?? null;
  // The site moved this page to another track: it must not play it (after the end, play() is held
  // already and the echo may ring on).
  if (started && !stopped && !ended && id && EXPECT && id !== EXPECT) stop();
  return {
    id,
    ad: adShowing(),
    loading: !player(),
    ready: !!v && target !== null && seeked && v.readyState >= 3 && !held && id === EXPECT,
    started,
    paused: v ? v.paused && !pending : true,
    ended,
    t: v ? v.currentTime : 0,
    rate: v ? v.playbackRate : 1,
    wall: wall(),
    error: errorOf(v),
    blocked,
    offset_ms: offsetMs,
    locked: recent.length >= LOCK_SYNCS && recent.slice(-LOCK_SYNCS).every(Boolean),
  };
};

window.__feathertuneDeck = Object.freeze({
  // The state; before `go`, where to wait: `t`, a few seconds before the main page's fade (again
  // when it moves by more than half a second, the main page having planned its fade again).
  poll: (t) => {
    const v = video();
    if (v && !started && Number.isFinite(t) && v.readyState >= 1 && !adNow()) {
      const want = Math.max(0, t);
      if (target === null || Math.abs(want - target) > 0.5) {
        target = want;
        seeked = false;
        v.currentTime = target;
      }
    }
    return state();
  },
  // Builds the graph and plays from `play_ms` (at once when that is past), silent until the swap.
  // `level`: the main page's volume; `chain`: its equalizer gains (dB), compressor and trim now.
  go: (o) => {
    const v = video();
    if (!v || started || adNow()) return false;
    ctx = new AudioContext({ latencyHint: 'playback' });
    const source = ctx.createMediaElementSource(v);
    let last = source;
    const link = (node) => {
      last.connect(node);
      last = node;
      return node;
    };
    const eq = Array.isArray(o?.chain?.eq) && o.chain.eq.length === EQ_FREQUENCIES.length ? o.chain.eq : [];
    eq.forEach((g, i) => {
      if (!Number.isFinite(g) || g === 0) return;
      const band = ctx.createBiquadFilter();
      band.type = i === 0 ? 'lowshelf' : i === EQ_FREQUENCIES.length - 1 ? 'highshelf' : 'peaking';
      band.frequency.value = EQ_FREQUENCIES[i];
      band.Q.value = EQ_Q;
      band.gain.value = num(g, -12, 12, 0);
      link(band);
    });
    const c = o?.chain?.comp;
    if (c) {
      const comp = ctx.createDynamicsCompressor();
      comp.threshold.value = num(c.threshold, -100, 0, 0);
      comp.knee.value = num(c.knee, 0, 40, 0);
      comp.ratio.value = num(c.ratio, 1, 20, 1);
      comp.attack.value = num(c.attack, 0, 1, 0.003);
      comp.release.value = num(c.release, 0, 1, 0.25);
      link(comp);
    }
    const trim = link(ctx.createGain());
    trim.gain.value = num(o?.chain?.trim, 0, MAX_LEVEL, 1);
    const bass = link(ctx.createBiquadFilter());
    bass.type = 'lowshelf';
    bass.frequency.value = BASS_HZ;
    bass.gain.value = 0;
    const sweep = link(ctx.createBiquadFilter());
    sweep.type = 'highpass';
    sweep.frequency.value = SWEEP_FROM_HZ;
    sweep.Q.value = 0.7;
    const fade = link(ctx.createGain());
    fade.gain.value = 0;
    const level = link(ctx.createGain());
    level.gain.value = levelOf(o?.level, v);
    level.connect(ctx.destination);
    // The echo: sweep → send → delay ⇄ feedback, delay → wet → level (connected at the swap, if used).
    const send = ctx.createGain();
    send.gain.value = 0;
    const delay = ctx.createDelay(2);
    const feedback = ctx.createGain();
    feedback.gain.value = ECHO_FEEDBACK;
    const wet = ctx.createGain();
    wet.gain.value = ECHO_WET;
    n = { bass, sweep, fade, level, send, delay, feedback, wet };
    started = true;
    const wait = (num(o?.play_ms, 0, Infinity, 0) - wall()) / 1000;
    if (wait > 0.05) {
      pending = true;
      onClock(ctx.currentTime + wait, begin);
    } else {
      begin();
    }
    return true;
  },
  // The main page's latest sample: its time `t` at `wall_ms` (this page's clock), its rate and output
  // latency. The heard positions are compared: one seek when far apart, else a small rate correction.
  sync: (s) => {
    const v = video();
    if (!v || !started || stopped || ended || frozen || pending || correcting || v.paused || v.readyState < 3) {
      recent = [];
      offsetMs = null;
      return state();
    }
    mainRate = num(s?.rate, 0.25, 4, 1);
    const latency = (ctx.baseLatency || 0) + (ctx.outputLatency || 0);
    const main = num(s?.t, 0, Infinity, 0) + ((wall() - num(s?.wall_ms, 0, Infinity, wall())) / 1000) * mainRate - num(s?.latency, 0, 1, 0);
    const offset = v.currentTime - latency - main;
    offsetMs = Math.round(offset * 10000) / 10;
    if (Math.abs(offset) > SEEK_OVER_S) {
      correcting = true;
      recent = [];
      v.playbackRate = mainRate;
      v.currentTime = main + SEEK_AHEAD_S * mainRate + latency;
      return state();
    }
    v.preservesPitch = true;
    v.playbackRate = mainRate * (1 - Math.max(-MAX_CORRECTION, Math.min(MAX_CORRECTION, offset * RATE_GAIN)));
    recent.push(Math.abs(offset) <= LOCK_S);
    if (recent.length > LOCK_SYNCS) recent.shift();
    return state();
  },
  // The swap at `at_ms`: up in `swap_ms` while the main page goes down, then the fade-out along `curve`
  // over `dur_ms` with the effects (`echo`, `filter`, `bass_swap`; `beat_ms` long). The corrections
  // stop FREEZE_S before it, at the main page's rate.
  swap: (o) => {
    if (!n || stopped) return false;
    const now = ctx.currentTime;
    const w = Math.max(now + 0.01, at(num(o?.at_ms, 0, Infinity, 0)));
    const start = w + num(o?.swap_ms, 0, 1000, 60) / 1000;
    const dur = num(o?.dur_ms, 50, 60000, 6000) / 1000;
    const beat = num(o?.beat_ms, 50, 2000, 500) / 1000;
    onClock(w - FREEZE_S, () => {
      frozen = true;
      const v = video();
      if (v) v.playbackRate = mainRate;
    });
    const g = n.fade.gain;
    g.cancelAndHoldAtTime(now);
    g.setValueAtTime(0, w);
    g.linearRampToValueAtTime(1, start);
    try {
      g.setValueCurveAtTime(fadeOut(o?.curve), start + 0.001, dur);
    } catch {
      g.linearRampToValueAtTime(0, start + dur);
    }
    if (o?.filter) {
      const f = n.sweep.frequency;
      f.setValueAtTime(SWEEP_FROM_HZ, start);
      f.exponentialRampToValueAtTime(SWEEP_TO_HZ, start + dur);
    }
    if (o?.bass_swap) {
      const half = start + dur / 2;
      n.bass.gain.setValueAtTime(0, half);
      n.bass.gain.linearRampToValueAtTime(BASS_CUT_DB, half + beat);
    }
    if (o?.echo) {
      n.delay.delayTime.value = Math.min(1.5, beat);
      n.sweep.connect(n.send).connect(n.delay).connect(n.feedback).connect(n.delay);
      n.delay.connect(n.wet).connect(n.level);
      n.send.gain.setTargetAtTime(1, start + 0.5 * dur, RAMP / 3);
      n.send.gain.setTargetAtTime(0, start + 0.85 * dur, RAMP / 3);
    }
    return true;
  },
  // The main page's volume changed.
  volume: (l) => {
    const v = video();
    if (n && v && !stopped) n.level.gain.setTargetAtTime(levelOf(l, v), ctx.currentTime, RAMP);
  },
  stop,
});
