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
  // The rows of a playlist page and of the queue (the mix buttons go between them).
  listRows: 'ytmusic-playlist-shelf-renderer ytmusic-responsive-list-item-renderer',
  queueRows: 'ytmusic-player-queue ytmusic-player-queue-item',
  rowLink: 'a[href*="watch?v="]',
  // The expanded player page: the layout's attribute while it is open, and the artwork's box.
  layout: 'ytmusic-app-layout',
  playerOpenAttr: 'player-page-open',
  artwork: 'ytmusic-player-page #player',
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
  if (bassCut.on) list.push(nodes.bass);
  if (sweep.on) list.push(nodes.sweep);
  return list;
};
// The fixed order of every effect node.
const order = () => [...nodes.eq, nodes.compressor, nodes.bass, nodes.sweep];

const neutralize = (node) => {
  if (node === nodes.sweep) {
    ramp(node.frequency, SWEEP_FROM_HZ);
  } else if (node === nodes.compressor) {
    ramp(node.threshold, 0);
    ramp(node.ratio, 1);
  } else {
    ramp(node.gain, 0);
  }
};
const target = (node) => {
  // The sweep's frequency and the bass cut are scheduled by the transition (armSweep, startOverlap).
  if (node === nodes.sweep || node === nodes.bass) return;
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
      // A mix's filter effect: a high-pass rising on the outgoing track.
      sweep: ctx.createBiquadFilter(),
      // The bass swap of an overlapping transition: the outgoing track's lows go down halfway.
      bass: ctx.createBiquadFilter(),
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
    nodes.sweep.type = 'highpass';
    nodes.sweep.frequency.value = SWEEP_FROM_HZ;
    nodes.sweep.Q.value = 0.7;
    nodes.bass.type = 'lowshelf';
    nodes.bass.frequency.value = BASS_HZ;
    nodes.bass.gain.value = 0;
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
// The filter effect: the high-pass rises from (neutral) to this over the fade.
const SWEEP_FROM_HZ = 20;
const SWEEP_TO_HZ = 1000;
// The bass swap (with the transition voice): a low shelf at 200 Hz, -24 dB.
const BASS_HZ = 200;
const BASS_CUT_DB = -24;
// The transition voice: how long the next track may take to start before it fades in by itself,
// how far ahead the take-over is scheduled and how long it crosses.
const HANDOFF_WAIT_MS = 5000;
const HANDOFF_AHEAD_MS = 300;
const HANDOFF_S = 0.15;
// The tempo effect goes back to 1.0 over 8 bars once this page plays the track.
const RATE_BACK_BEATS = 32;
// A command from Rust older than this is dropped (the page reloaded meanwhile, say).
const COMMAND_TTL_MS = 5000;
// A preview starts this long before the transition.
const PREVIEW_LEAD_S = 3;

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
// The fade of the current track: { out, dur, curve, echo, filter, to, in } in video seconds, or null.
let fade = null;
let armed = false;
// The fade-out reached its end (or was cut short by a track change).
let faded = false;
let ownSeek = false;
const echo = { on: false, until: 0 };
const sweep = { on: false };
const bassCut = { on: false };
// An overlapping transition through the transition voice (deck.rs), from its `overlap` command
// until this page plays the next track: { to, at_ms, waiting, since, done_at }.
let overlap = null;
// The tempo effect after a take-over: { from, start, len } in video seconds.
let rateBack = null;
// A mix's transition being previewed from the Mix tab (a pair, while its `from` track plays).
let preview = null;
// After a mix's fade: where its incoming track starts, { from, id, at }.
let pendingIn = null;
let lastSeq = 0;

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

// The mix's transition out of this track (mixes.rs `plan`: a pair, a playlist's defaults or auto),
// or the one being previewed.
const transition = () => {
  if (preview?.from === trackId) return preview;
  const t = data?.transition;
  return t && t.from === trackId ? t : null;
};

// The fade for this track: the mix's transition, else the global settings.
const planFade = (v) => {
  const c = data?.crossfade;
  const m = transition();
  const ms = m ? m.duration_ms : c?.duration_ms;
  if (!on('crossfade') || !ms || ad || repeatOne(v) || !Number.isFinite(v.duration)) return null;
  const dur = ms / 1000;
  // Before the final silence, once it is known.
  const tail = facts()?.tail_ms;
  const end = tail && tail / 1000 < v.duration ? tail / 1000 : v.duration;
  let out = end - dur;
  if (m && Number.isFinite(m.out_ms)) out = Math.min(m.out_ms / 1000, v.duration - dur);
  else if (end < 3 * dur) return null;
  if (out < 0) return null;
  return {
    out,
    dur,
    curve: CURVES[m ? m.curve : c.curve] || CURVES.equal_power,
    echo: m ? !!m.effects?.echo : !!c.echo,
    filter: !!m?.effects?.filter,
    to: m?.to ?? null,
    in: m ? (m.in_ms || 0) / 1000 : 0,
  };
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
  if (fade.filter) armSweep(begin, p, length);
};

// The filter: the high-pass enters the chain and rises along the fade (exponentially, as heard).
const armSweep = (begin, p, length) => {
  if (!sweep.on) {
    sweep.on = true;
    apply();
  }
  const f = nodes.sweep.frequency;
  const now = ctx.currentTime;
  f.cancelAndHoldAtTime(now);
  f.setValueAtTime(Math.max(SWEEP_FROM_HZ, f.value), now);
  f.exponentialRampToValueAtTime(SWEEP_FROM_HZ * Math.pow(SWEEP_TO_HZ / SWEEP_FROM_HZ, p), begin);
  f.exponentialRampToValueAtTime(SWEEP_TO_HZ, begin + Math.max(0.02, length));
};
// The high-pass back to neutral and out of the chain.
const stopSweep = () => {
  if (!sweep.on) return;
  sweep.on = false;
  apply();
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
  nodes.sweep.frequency.cancelAndHoldAtTime(ctx.currentTime);
  armed = false;
};
// Back to full level in 30 ms, nothing scheduled.
const cancelFade = () => {
  armed = false;
  ramp(nodes.master.gain, 1);
  stopEcho(false);
  stopSweep();
};

const inWindow = (t) => fade && t >= fade.out - ARM_BEFORE_S && t < fade.out + fade.dur;

// The next track starts: it fades in if the last one faded out (a short fade if it was cut off).
const startTrack = (v, id) => {
  if (analysis && analysis.id !== id) stopAnalysis(analysis.t >= analysis.duration - 1.5);
  const cut = armed && !faded;
  const prev = nodes.master.gain.value;
  // A mix's incoming track: where it starts (see enterAt).
  if (pendingIn && pendingIn.id !== id) pendingIn = null;
  trackId = id;
  armed = false;
  faded = false;
  preview = null;
  stopSweep();
  stopBassCut();
  if (rateBack) {
    rateBack = null;
    v.playbackRate = 1;
  }
  resetQuiet();
  fade = planFade(v);
  if (overlap && !ad) {
    if (id === overlap.to && !overlap.waiting) {
      // The transition voice plays this track's start: silent here until the take-over (handoff).
      const g = nodes.master.gain;
      g.cancelAndHoldAtTime(ctx.currentTime);
      g.setValueAtTime(0, ctx.currentTime);
      overlap.waiting = true;
      overlap.since = Date.now();
      quiet.startDone = true;
      return;
    }
    if (id !== overlap.to) overlap = null;
  }
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
  if (pendingIn && id) enterAt(v, id);
  if (overlap) overlapStep(v);
  if (rateBack) rateStep(v);
  if (!fade && on('crossfade')) fade = planFade(v);
  const t = v.currentTime;
  if (analysis && Math.abs(analysis.t - t) > 0.5) {
    // Out of step (buffering): the envelope goes on from here, the beat window is lost.
    if (inBeatWindow(analysis)) analysis.beatOk = false;
    analysis.t = t;
  }
  // A track that starts under the transition voice is entered late: not measured.
  if (id && !overlap) startAnalysis(v, id);
  skipSilence(v, t);
  // Repeat of the same track after its fade (a one-track queue on repeat-all): back in.
  if (faded && t < 2) startTrack(v, id);
  if (!fade) return;
  if (!armed && inWindow(t)) arm(v);
  // The fade is over: let the track end by itself (unless the fade already ends with it).
  if (armed && !faded && t >= fade.out + fade.dur - 0.05) {
    faded = true;
    if (!overlap && fade.to && fade.in > 0) pendingIn = { from: trackId, id: fade.to, at: fade.in };
    if (fade.out + fade.dur < v.duration - 0.1) seekTo(v, v.duration - 0.05);
  }
};

// A mix's incoming track starts at its in point, once its duration is known (before the analysis
// would start: a track entered late is not measured).
const enterAt = (v, id) => {
  if (id === pendingIn.from) return;
  if (id !== pendingIn.id) return void (pendingIn = null);
  if (!Number.isFinite(v.duration)) return;
  const at = pendingIn.at;
  pendingIn = null;
  if (at < v.duration - 5 && v.currentTime < at) seekTo(v, at);
};

// Commands from Rust (mixes.rs): each runs once, and only while fresh.
const runCommand = (c) => {
  if (!c || !(c.seq > lastSeq)) return;
  lastSeq = c.seq;
  if (!(Date.now() - c.issued_ms < COMMAND_TTL_MS)) return;
  if (c.preview) previewMix(c.preview);
  if (c.overlap) startOverlap(c.overlap);
  if (c.overlap_off) overlapOff();
  if (c.handoff) handoff(c.handoff);
};

// --- the transition voice (deck.rs) ---
//
// Rust plays the start of the next track in a hidden second webview during the fade (an overlap
// instead of a fade-out then a fade-in). This page only: fades out as usual; keeps the next track
// silent when the site starts it (`waiting`); on `handoff`, seeks it to where the voice is and
// crosses over in 150 ms at a time Rust then gives the voice (`done_at`). Anything late or wrong
// falls back to the one-player fade by itself.

const wall = () => performance.timeOrigin + performance.now();
// A wall-clock time on the audio clock.
const ctxAt = (wallMs) => {
  const ts = ctx.getOutputTimestamp?.();
  if (ts && ts.performanceTime > 0) return ts.contextTime + (wallMs - performance.timeOrigin - ts.performanceTime) / 1000;
  return ctx.currentTime + (wallMs - wall()) / 1000;
};

// `overlap` { to, at_ms, dur_ms, bass_swap, beat_ms }: the voice starts `to` at `at_ms` (this
// fade's start). With the bass swap, this track's lows go down halfway, over one beat.
const startOverlap = (o) => {
  if (!nodes || ad || !fade || fade.to !== o.to) return;
  overlap = { to: o.to, waiting: false, since: 0, done_at: null, sync: null, tries: 0 };
  pendingIn = null;
  if (!o.bass_swap) return;
  bassCut.on = true;
  apply();
  const now = ctx.currentTime;
  const half = Math.max(now, ctxAt(o.at_ms + o.dur_ms / 2));
  const g = nodes.bass.gain;
  g.cancelAndHoldAtTime(now);
  g.setValueAtTime(0, half);
  g.linearRampToValueAtTime(BASS_CUT_DB, half + Math.max(0.05, o.beat_ms / 1000));
};
const stopBassCut = () => {
  if (!bassCut.on) return;
  bassCut.on = false;
  apply();
};
// The voice failed or was cut: back to the one-player fade (in 300 ms if the next track was held).
const overlapOff = () => {
  if (!overlap) return;
  const held = overlap.waiting && !overlap.done_at;
  overlap = null;
  stopBassCut();
  if (held && nodes && !ad) {
    const g = nodes.master.gain;
    g.cancelAndHoldAtTime(ctx.currentTime);
    g.setTargetAtTime(1, ctx.currentTime, SKIP_FADE_IN_S / 3);
  }
};
// `handoff` { t, wall_ms, rate }: where the voice was at `wall_ms`. This page plays at its rate
// (the tempo effect) and seeks to where the voice will be.
const handoff = (h) => {
  const v = video;
  if (!v || !overlap?.waiting || overlap.done_at || ad || currentId() !== overlap.to) return;
  overlap.sync = { t: h.t, wall: h.wall_ms, rate: h.rate > 0.9 && h.rate < 1.1 ? h.rate : 1 };
  if (overlap.sync.rate !== 1) {
    v.preservesPitch = true;
    v.playbackRate = overlap.sync.rate;
  }
  syncTo(v);
};
const voiceAt = (wallMs) => overlap.sync.t + ((wallMs - overlap.sync.wall) / 1000) * overlap.sync.rate;
// A seek lands a little later than asked: aim 250 ms ahead, then check (at most three tries).
const syncTo = (v) => {
  overlap.tries++;
  seekTo(v, voiceAt(wall() + 250));
};
// On every timeupdate while an overlap is on: the take-over once in step, or the fallback.
const overlapStep = (v) => {
  const o = overlap;
  if (o.done_at) {
    if (wall() > o.done_at + 2000) overlap = null;
    return;
  }
  if (o.waiting && Date.now() - o.since > HANDOFF_WAIT_MS) return overlapOff();
  if (!o.sync || ownSeek || v.readyState < 3) return;
  const drift = v.currentTime - voiceAt(wall());
  if (Math.abs(drift) > 0.08 && o.tries < 3) return syncTo(v);
  takeOver(v);
};
const takeOver = (v) => {
  const at = wall() + HANDOFF_AHEAD_MS;
  const c = Math.max(ctx.currentTime, ctxAt(at));
  const g = nodes.master.gain;
  g.cancelAndHoldAtTime(ctx.currentTime);
  g.setValueAtTime(0, c);
  g.linearRampToValueAtTime(1, c + HANDOFF_S);
  overlap.done_at = at;
  if (v.playbackRate !== 1) {
    const bpm = facts()?.bpm;
    rateBack = { from: v.playbackRate, start: v.currentTime, len: bpm ? (RATE_BACK_BEATS * 60) / bpm : 16 };
  }
};
// The tempo effect back to 1.0, in steps on timeupdate (a rate is not an AudioParam).
const rateStep = (v) => {
  const p = Math.min(1, (v.currentTime - rateBack.start) / rateBack.len);
  const rate = p >= 1 ? 1 : rateBack.from + (1 - rateBack.from) * p;
  if (Math.abs(v.playbackRate - rate) > 0.002 || rate === 1) v.playbackRate = rate;
  if (rate === 1) rateBack = null;
};
// The Mix tab's preview: the pair's transition, from a few seconds before it (only on its `from` track).
const previewMix = (pair) => {
  const v = video;
  if (!v || !nodes || ad || v.paused || pair.from !== trackId) return;
  if (armed) cancelFade();
  faded = false;
  preview = pair;
  fade = planFade(v);
  if (fade) seekTo(v, Math.max(0, fade.out - PREVIEW_LEAD_S));
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
  visUpdate();
};
// An ad: nothing stays scheduled (apply() brings every parameter back to neutral).
const onAd = (now) => {
  if (!now || !nodes) return;
  armed = false;
  fade = null;
  preview = null;
  pendingIn = null;
  overlap = null;
  sweep.on = false;
  bassCut.on = false;
  // The tempo effect never touches an ad.
  if (rateBack) {
    rateBack = null;
    if (video) video.playbackRate = 1;
  }
  nodes.master.gain.cancelAndHoldAtTime(ctx.currentTime);
  nodes.sweep.frequency.cancelAndHoldAtTime(ctx.currentTime);
  stopEcho(true);
  stopAnalysis(false);
};

// --- mix buttons ---
//
// In a playlist whose mix is on (data.mixes), a small button between two rows (of the playlist page
// and of the queue) opens the Mix tab on that pair through `mix_open`; it is marked when the pair was
// edited. The buttons are redrawn when the data changes, on the site's navigation and when the rows
// change (an observer per list, settled for 300 ms), never on a timer.

const BUTTON_CLASS = 'feathertune-mix';
const rowId = (row) => {
  const id = row.data?.videoId;
  if (typeof id === 'string') return id;
  const href = row.querySelector(SELECTORS.rowLink)?.getAttribute('href') || '';
  return new URLSearchParams(href.split('?')[1] || '').get('v');
};
const pageList = () => new URLSearchParams(location.search).get('list');

const renderButtons = () => {
  const list = pageList();
  const edited = list && data?.mixes?.[list];
  for (const selector of [SELECTORS.listRows, SELECTORS.queueRows]) {
    const rows = [...document.querySelectorAll(selector)];
    rows.forEach((row, i) => {
      let button = row.querySelector(`:scope > .${BUTTON_CLASS}`);
      const from = rowId(row);
      const to = rows[i + 1] && rowId(rows[i + 1]);
      const valid = (x) => typeof x === 'string' && /^[A-Za-z0-9_-]{11}$/.test(x);
      if (!Array.isArray(edited) || !valid(from) || !valid(to) || from === to) {
        button?.remove();
        return;
      }
      if (!button) {
        button = document.createElement('button');
        button.className = BUTTON_CLASS;
        button.type = 'button';
        button.addEventListener('mousedown', (e) => e.stopPropagation(), true);
        button.addEventListener('click', (e) => {
          e.preventDefault();
          e.stopPropagation();
          const { list, from, to } = button.dataset;
          window.__TAURI_INTERNALS__?.invoke('mix_open', { list, from, to }).catch(() => {});
        });
        row.append(button);
      }
      const mark = edited.includes(`${from}>${to}`);
      Object.assign(button.dataset, { list, from, to });
      button.classList.toggle('edited', mark);
      button.textContent = mark ? '⇄ Edited' : '⇄ Auto';
      button.title = 'Edit the transition between these tracks';
    });
  }
  watchRows();
};

let rowsTimer = 0;
// Only the rows' parents are watched (the buttons go inside the rows, so they do not count).
const rowsObserver = new MutationObserver(() => {
  clearTimeout(rowsTimer);
  rowsTimer = setTimeout(renderButtons, 300);
});
const watchedRows = new WeakSet();
const watchRows = () => {
  for (const selector of [SELECTORS.listRows, SELECTORS.queueRows]) {
    const parent = document.querySelector(selector)?.parentElement;
    if (!parent || watchedRows.has(parent)) continue;
    watchedRows.add(parent);
    rowsObserver.observe(parent, { childList: true });
  }
};
document.addEventListener('yt-navigate-finish', () => setTimeout(renderButtons, 300));

// --- visualizer ---
//
// Bars or a wave under the artwork on the expanded player page, in the theme's accent color. One
// requestAnimationFrame loop, only while the player page is open, the page visible, the window
// focused and the music playing (no ad); the analyser is connected only meanwhile. Stopped, the
// last frame stays; an ad hides it.

const VIS_BARS = 48;
const vis = { canvas: null, g: null, raf: 0, bytes: null, color: '#fff', frames: 0 };
const playerOpen = () => !!document.querySelector(SELECTORS.layout)?.hasAttribute(SELECTORS.playerOpenAttr);
const visWanted = () =>
  on('visualizer') && !!nodes && !ad && !!video && !video.paused && playerOpen() && document.visibilityState === 'visible' && document.hasFocus();

const visCanvas = () => {
  const box = document.querySelector(SELECTORS.artwork);
  if (!box) return null;
  if (!vis.canvas) {
    vis.canvas = document.createElement('canvas');
    vis.canvas.className = 'feathertune-visualizer';
    vis.canvas.setAttribute('aria-hidden', 'true');
    vis.g = vis.canvas.getContext('2d');
  }
  if (vis.canvas.parentElement !== box) box.append(vis.canvas);
  return vis.canvas;
};
const visColor = () => {
  const accent = getComputedStyle(document.documentElement).getPropertyValue('--feathertune-accent').trim();
  vis.color = accent || '#fff';
};

const visUpdate = () => {
  const want = visWanted();
  if (want && !vis.raf && visCanvas()) {
    useAnalyser('visualizer', true);
    visColor();
    vis.canvas.hidden = false;
    vis.raf = requestAnimationFrame(visFrame);
  } else if (!want && vis.raf) {
    cancelAnimationFrame(vis.raf);
    vis.raf = 0;
    useAnalyser('visualizer', false);
  }
  if (vis.canvas && (ad || !on('visualizer') || !playerOpen())) vis.canvas.hidden = true;
};

const visFrame = () => {
  vis.raf = 0;
  if (!visWanted()) return visUpdate();
  const c = vis.canvas;
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(c.clientWidth * dpr);
  const h = Math.round(c.clientHeight * dpr);
  if (c.width !== w || c.height !== h) {
    c.width = w;
    c.height = h;
  }
  // The theme may change with the track: read again now and then.
  if (++vis.frames % 120 === 0) visColor();
  const a = nodes.analyser;
  vis.bytes ||= new Uint8Array(a.frequencyBinCount);
  const g = vis.g;
  g.clearRect(0, 0, w, h);
  g.fillStyle = vis.color;
  g.strokeStyle = vis.color;
  if (data?.visualizer === 'wave') {
    a.getByteTimeDomainData(vis.bytes);
    g.lineWidth = 2 * dpr;
    g.globalAlpha = 0.9;
    g.beginPath();
    const n = vis.bytes.length;
    for (let i = 0; i < n; i += 4) g.lineTo((i / (n - 1)) * w, (vis.bytes[i] / 255) * h);
    g.stroke();
  } else {
    a.getByteFrequencyData(vis.bytes);
    // Log-spaced bands from ~40 Hz to ~16 kHz.
    const bin = ctx.sampleRate / a.fftSize;
    const gap = 2 * dpr;
    const bw = (w - gap * (VIS_BARS - 1)) / VIS_BARS;
    g.globalAlpha = 0.85;
    for (let i = 0; i < VIS_BARS; i++) {
      const lo = Math.max(1, Math.floor((40 * Math.pow(400, i / VIS_BARS)) / bin));
      const hi = Math.max(lo + 1, Math.floor((40 * Math.pow(400, (i + 1) / VIS_BARS)) / bin));
      let peak = 0;
      for (let k = lo; k < hi && k < vis.bytes.length; k++) peak = Math.max(peak, vis.bytes[k]);
      const bh = Math.max(dpr, (peak / 255) * h);
      g.fillRect(i * (bw + gap), h - bh, bw, bh);
    }
  }
  g.globalAlpha = 1;
  vis.raf = requestAnimationFrame(visFrame);
};

let layoutWatched = null;
const layoutObserver = new MutationObserver(() => visUpdate());
const watchLayout = () => {
  const layout = document.querySelector(SELECTORS.layout);
  if (!layout || layout === layoutWatched) return;
  layoutObserver.disconnect();
  layoutObserver.observe(layout, { attributes: true, attributeFilter: [SELECTORS.playerOpenAttr] });
  layoutWatched = layout;
};
document.addEventListener('visibilitychange', visUpdate);
window.addEventListener('focus', visUpdate);
window.addEventListener('blur', visUpdate);

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
  watchLayout();
  if (e.type === 'play' || e.type === 'pause' || e.type === 'playing') visUpdate();
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
      if (analysis && data?.facts?.current?.id === analysis.id) stopAnalysis(false);
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
    runCommand(d?.command);
    renderButtons();
    visUpdate();
  },
  // The transition voice (deck.rs) reads this through ExecuteScript.
  deck: () => {
    const v = video;
    return {
      id: currentId() ?? null,
      t: v ? v.currentTime : 0,
      rate: v ? v.playbackRate : 1,
      paused: v ? v.paused : true,
      ad,
      out: fade && !ad && nodes ? fade.out : null,
      dur: fade ? fade.dur : null,
      to: fade?.to ?? null,
      wall: wall(),
      waiting: !!overlap?.waiting,
      done_at: overlap?.done_at ?? null,
      error: !!v?.error,
    };
  },
  // --measure: one line of state.
  probe: () => {
    const state = off ? 'off' : ctx ? ctx.state : 'none';
    return `audio=${state} ctx_ms=${ctx ? Math.round(ctx.currentTime * 1000) : 0} worklet=${worklet} ad=${ad}`;
  },
});
