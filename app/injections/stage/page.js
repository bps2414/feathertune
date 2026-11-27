// Stage (plugin `stage`, see src/stage.rs): plays the script Rust wrote for the track as a cinematic
// full-screen show over the site. Rust sends `{ open: true, id, artwork, backdrop, halo, script, lines,
// translation, texts }` while open and `{ open: false }` when closed; display fonts come once via
// `stageFont`. The page asks Rust for nothing.
// - Never over an ad: while `feathertune.ad` is up the layer hides at once and the site shows whole.
// - Under the layer the site's app is `visibility: hidden` and `feathertune.covered` is set.
// - Bulletproof covers: zero detached flying duplicate cover; explicit square frame with aspect-ratio crop
//   for 4:3 letterboxed video thumbnails, smooth decode fade-in, clean fallback on error, and in-place
//   updates when lyrics/backdrop arrive for the same track.
// - Typography varies by Mood (`cinema`, `editorial`, `poster`, `nocturne`) AND by section role (intimate
//   verse vs. driven verse vs. chorus vs. title), with zero per-word horizontal axis stretching and zero
//   duplicate neon glow layers.
// - One animation frame loop while open, visible, playing and not showing an ad. Paused: draws one frame
//   and pauses all animations. Light effects (`html[data-feathertune-light]`) keep only crisp fades/cuts.
if (window.top !== window) return;

const LEAD_MS = 250;
const IDLE_MS = 2200;
const KINDS = ['opening', 'close', 'editorial', 'chorus', 'instrumental', 'credits'];
const TEXT_KINDS = new Set(['close', 'editorial', 'chorus']);
const RELIEF_KINDS = new Set(['opening', 'instrumental', 'credits']);
const LAYOUTS = ['focus', 'editorial', 'hero', 'split', 'anthem', 'climb', 'echo', 'crescendo'];
const ENTERS = ['illuminate', 'curtain', 'glide', 'focus', 'impact'];
const EXITS = ['drift_up', 'dissolve', 'curtain_down', 'recede'];
const ENTRIES = ['cut', 'lift', 'breath', 'frame', 'return'];
const CAMERAS = ['push', 'pull', 'left', 'right', 'rise'];

const DEFAULT_SHOT = {
  layout: 'focus',
  x: 0,
  y: 0,
  hero: -1,
  enter: 'illuminate',
  exit: 'dissolve',
  level: 0,
  italic: false,
  singer: 0,
  answer: false,
  cont: false,
};

const EXIT_MS = { drift_up: 460, dissolve: 440, curtain_down: 420, recede: 520, fade: 360 };
const HANDOFF_MS = 240;
const SEEN_MS = 180;
const FRAME_OUT_MS = 650;
const BLEND_MS = { MIN: 900, MAX: 1800, NO_GRID: 1100 };
const PUSH_SHARE = 0.24;
const CLIMB_TAIL_MS = 700;
const FONT_WAIT_MS = 350;
const LIVE_DB = { FLOOR: -44, RANGE: 36, FOLLOW: 0.1 };
const EASE_FILM = 'cubic-bezier(0.22, 1, 0.36, 1)';
const EASE_EXIT = 'cubic-bezier(0.4, 0, 0.2, 1)';
const MAX_LINES = 2000;
const MAX_LIGHT = 9000;
const SAFE = { x: 0.08, y: 0.12, w: 0.84, h: 0.66 };
const LATIN = /^[ -ɏḀ-ỿ -⁯]+$/;
const DATA_PNG = /^data:image\/png;base64,[A-Za-z0-9+/=]{1,600000}$/;
const ART_HOSTS = ['googleusercontent.com', 'ggpht.com', 'ytimg.com'];
const ALLOWED_FAMILIES = new Set(['ft-serif', 'ft-sans', 'ft-syne', 'ft-poster']);

const TEXTS = {
  leave: 'Leave the stage',
  esc: 'Esc to leave',
  play: 'Play',
  pause: 'Pause',
  instrumental: 'Instrumental',
  listened: 'You listened to',
  next: 'Up next',
};

// Section-aware typographic art direction per Mood: fixed natural proportions (zero word-width stretching).
const MOODS = {
  cinema: {
    title: { face: "'ft-serif', Georgia, serif", weight: '400', fv: 'normal', track: '-0.015em', kase: 'asis' },
    close: { face: "'ft-serif', Georgia, serif", fv: 'normal', smallFv: "'wght' 420", smallFace: "'ft-sans', sans-serif", lh: 1.06, gap: 0.26, track: -0.01, kase: 'asis', heroCase: 'asis' },
    editorial: { face: "'ft-sans', sans-serif", fv: "'wght' 480", smallFv: "'wght' 380", lh: 1.12, gap: 0.25, track: -0.02, kase: 'asis', heroCase: 'asis' },
    chorus: { face: "'ft-serif', Georgia, serif", fv: 'normal', smallFv: "'wght' 460", smallFace: "'ft-sans', sans-serif", lh: 1.02, gap: 0.25, track: -0.015, kase: 'asis', heroCase: 'asis' },
  },
  editorial: {
    title: { face: "'ft-sans', sans-serif", weight: '700', fv: "'wght' 700", track: '-0.03em', kase: 'asis' },
    close: { face: "'ft-sans', sans-serif", fv: "'wght' 380", smallFv: "'wght' 340", lh: 1.14, gap: 0.26, track: -0.012, kase: 'asis', heroCase: 'asis' },
    editorial: { face: "'ft-sans', sans-serif", fv: "'wght' 620", smallFv: "'wght' 420", lh: 1.08, gap: 0.24, track: -0.025, kase: 'asis', heroCase: 'asis' },
    chorus: { face: "'ft-sans', sans-serif", fv: "'wght' 780", smallFv: "'wght' 500", lh: 1.02, gap: 0.22, track: -0.032, kase: 'asis', heroCase: 'asis' },
  },
  poster: {
    title: { face: "'ft-poster', sans-serif", weight: '800', fv: "'wdth' 80, 'wght' 820", track: '-0.01em', kase: 'upper' },
    close: { face: "'ft-sans', sans-serif", fv: "'wght' 440", smallFv: "'wght' 360", lh: 1.14, gap: 0.26, track: -0.015, kase: 'asis', heroCase: 'asis' },
    editorial: { face: "'ft-poster', sans-serif", fv: "'wdth' 86, 'wght' 700", smallFv: "'wdth' 96, 'wght' 450", lh: 0.96, gap: 0.22, track: -0.01, kase: 'asis', heroCase: 'upper' },
    chorus: { face: "'ft-poster', sans-serif", fv: "'wdth' 78, 'wght' 860", smallFv: "'wdth' 92, 'wght' 520", lh: 0.92, gap: 0.2, track: -0.01, kase: 'upper', heroCase: 'upper' },
  },
  nocturne: {
    title: { face: "'ft-syne', sans-serif", weight: '700', fv: "'wght' 720", track: '-0.025em', kase: 'asis' },
    close: { face: "'ft-sans', sans-serif", fv: "'wght' 350", smallFv: "'wght' 320", lh: 1.15, gap: 0.27, track: -0.01, kase: 'asis', heroCase: 'asis' },
    editorial: { face: "'ft-syne', sans-serif", fv: "'wght' 620", smallFv: "'wght' 480", lh: 1.06, gap: 0.25, track: -0.02, kase: 'asis', heroCase: 'asis' },
    chorus: { face: "'ft-syne', sans-serif", fv: "'wght' 780", smallFv: "'wght' 540", lh: 1.0, gap: 0.24, track: -0.025, kase: 'asis', heroCase: 'asis' },
  },
};

const st = {
  open: false,
  ad: false,
  raf: 0,
  still: false,
  v: null,
  trackId: '',
  key: '',
  sec: -1,
  j: -1,
  block: null,
  next: null,
  stageJob: 0,
  leaving: [],
  aside: null,
  // Active non-text scene `{ node, kind, artUrl, coverEl, haloEl, upNextEl }` and any outgoing scene.
  scene: null,
  sceneOut: null,
  outSceneNode: null,
  count: null,
  now: 0,
  resync: false,
  anims: new Set(),
  // Smoothed energy & gentle lumen pulse (never jagged or strobing).
  e: 0.45,
  smoothE: 0.45,
  lumenPeak: 0,
  live: false,
  // Active A/B background slot (0 or 1) and last applied backdrop URL.
  bgSlot: 0,
  bgUrl: '',
  fonts: false,
  opened: 0,
  total: 0,
  relief: null,
  idle: 0,
  resize: 0,
  shown: {},
};
let ui = null;

const num = (x) => typeof x === 'number' && Number.isFinite(x);
const text = (x, max = 300) => (typeof x === 'string' ? x.slice(0, max) : '');
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const smooth = (x) => x * x * (3 - 2 * x);
const video = () => document.querySelector('video');
const lightFx = () => !!feathertune.perf?.light();
const fmt = (s) => {
  const t = Math.max(0, Math.floor(s || 0));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
};
const cased = (s, kase) => (kase === 'upper' ? s.toLocaleUpperCase() : kase === 'lower' ? s.toLocaleLowerCase() : s);
const hash = (s) => [...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
const range = (a, b) => Array.from({ length: Math.max(0, b - a) }, (_, k) => a + k);

const el = (tag, cls, parent, content) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (content != null) e.textContent = content;
  parent?.append(e);
  return e;
};
const SVG = 'http://www.w3.org/2000/svg';
const svgEl = (tag, attrs, parent) => {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  parent?.append(e);
  return e;
};

const validArtUrl = (src) => {
  if (typeof src !== 'string' || !src) return '';
  try {
    const u = new URL(src);
    const h = u.hostname.toLowerCase();
    return u.protocol === 'https:' && ART_HOSTS.some((d) => h === d || h.endsWith(`.${d}`)) ? u.href : '';
  } catch {
    return '';
  }
};

const fallbackArtwork = () => {
  const list = navigator.mediaSession?.metadata?.artwork;
  const src = list?.length ? list[list.length - 1].src : '';
  return validArtUrl(src);
};

// Validate everything coming from Rust before it touches the DOM.
const check = (d) => {
  const s = d?.script;
  if (!s || !num(s.duration_ms) || !Array.isArray(s.sections) || !Array.isArray(d.lines)) return null;
  const timed = (x) => {
    const ws = Array.isArray(x)
      ? x
          .filter((w) => Array.isArray(w) && num(w[0]) && typeof w[1] === 'string')
          .slice(0, 200)
          .map(([ms, t]) => ({ ms, text: t.slice(0, 120) }))
      : null;
    return ws?.length ? ws : null;
  };
  const singer = (x) => (Number.isInteger(x) ? clamp(x, 0, 2) : 0);
  const lines = d.lines.slice(0, MAX_LINES).map((l) => {
    if (!Array.isArray(l)) return { ms: 0, text: '', words: null, singer: 0, bg: null };
    return { ms: num(l[0]) ? l[0] : 0, text: text(l[1]), words: timed(l[2]), singer: singer(l[3]), bg: timed(l[4]) };
  });
  const n = lines.length;
  const sign = (x) => (num(x) ? clamp(Math.round(x), -1, 1) : 0);
  const shot = (x) => ({
    layout: LAYOUTS.includes(x?.layout) ? x.layout : 'focus',
    x: sign(x?.x),
    y: sign(x?.y),
    hero: Number.isInteger(x?.hero) && x.hero >= 0 ? x.hero : -1,
    enter: ENTERS.includes(x?.enter) ? x.enter : 'illuminate',
    exit: EXITS.includes(x?.exit) ? x.exit : 'dissolve',
    level: Number.isInteger(x?.level) ? clamp(x.level, 0, 3) : 0,
    italic: x?.italic === true,
    singer: singer(x?.singer),
    answer: x?.answer === true,
    cont: x?.cont === true,
  });
  const sections = s.sections
    .filter((x) => x && KINDS.includes(x.kind) && num(x.from_ms) && num(x.to_ms))
    .slice(0, 400)
    .map((x) => {
      const ls = Array.isArray(x.lines) ? x.lines : [];
      const shots = Array.isArray(x.shots) ? x.shots : [];
      const pairs = ls.map((i, j) => [i, shot(shots[j])]).filter(([i]) => Number.isInteger(i) && i >= 0 && i < n);
      return {
        kind: x.kind,
        from: x.from_ms,
        to: x.to_ms,
        lines: pairs.map((p) => p[0]),
        shots: pairs.map((p) => p[1]),
        camera: CAMERAS.includes(x.camera) ? x.camera : 'push',
        entry: ENTRIES.includes(x.entry) ? x.entry : 'cut',
        chorus: Number.isInteger(x.chorus_no) ? clamp(x.chorus_no, 0, 99) : 0,
      };
    });
  if (!sections.length) return null;
  const grid = s.grid && num(s.grid.bpm) && s.grid.bpm > 0 && num(s.grid.offset_ms) ? { beat: 60000 / s.grid.bpm, off: s.grid.offset_ms } : null;
  const light = Float32Array.from(Array.isArray(s.light) ? s.light.slice(0, MAX_LIGHT) : [], (x) => (num(x) ? clamp(x, 0, 255) / 255 : 0));
  const perLine = (list, f) => lines.map((_, i) => f(Array.isArray(list) ? list[i] : undefined, i));
  const ends = perLine(s.ends, (x, i) => (num(x) && x > lines[i].ms ? x : (lines[i + 1]?.ms ?? lines[i].ms + 4000)));
  const translation = Array.isArray(d.translation) ? perLine(d.translation, (x) => text(x)) : null;
  const c = s.credits || {};
  const next = c.next && typeof c.next.title === 'string' ? { title: text(c.next.title, 200), artist: text(c.next.artist, 200) } : null;
  const t = d.texts || {};
  const texts = Object.fromEntries(Object.entries(TEXTS).map(([k, en]) => [k, text(t[k], 80) || en]));
  texts.scenes = Object.fromEntries(KINDS.map((k) => [k, text(t.scenes?.[k], 40) || k]));
  return {
    id: text(d.id, 64),
    artwork: validArtUrl(d.artwork),
    backdrop: typeof d.backdrop === 'string' && DATA_PNG.test(d.backdrop) ? d.backdrop : null,
    halo: typeof d.halo === 'string' && DATA_PNG.test(d.halo) ? d.halo : null,
    duration: s.duration_ms,
    sections,
    mood: Object.hasOwn(MOODS, s.mood) ? s.mood : 'cinema',
    duet: s.duet === true,
    grid,
    light,
    lines,
    ends,
    translation,
    credits: { title: text(c.title, 200), artist: text(c.artist, 200), album: text(c.album, 200), next },
    texts,
  };
};

const wordsOf = (i) => {
  const { lines, ends } = st.v;
  const l = lines[i];
  if (l.words) return l.words.map((w) => ({ ms: w.ms, text: w.text.trim() })).filter((w) => w.text);
  const parts = l.text.match(/\S+/g) || [];
  const span = Math.max(300, ends[i] - l.ms) * 0.85;
  const total = parts.reduce((a, p) => a + p.length, 0) || 1;
  let at = l.ms;
  return parts.map((p) => {
    const w = { ms: Math.round(at), text: p };
    at += (span * p.length) / total;
    return w;
  });
};

const press = (button, f) => {
  button.type = 'button';
  button.tabIndex = -1;
  button.addEventListener('mousedown', (e) => e.preventDefault());
  button.addEventListener('click', f);
};

const build = () => {
  const root = el('div');
  root.id = 'feathertune-stage';
  root.hidden = true;
  // Dual-layer A/B Gaussian cover canvas for smooth crossfades across tracks.
  const backdrop = el('div', 'ft-backdrop', root);
  const bgA = el('div', 'ft-bg-layer', backdrop);
  const bgB = el('div', 'ft-bg-layer', backdrop);
  const orbs = el('div', 'ft-orbs', root);
  el('div', 'ft-orb ft-orb-a', orbs);
  el('div', 'ft-orb ft-orb-b', orbs);
  const grade = el('div', 'ft-grade', root);
  const lumen = el('div', 'ft-lumen', root);
  const camera = el('div', 'ft-camera', root);
  const lines = el('div', 'ft-lines', camera);
  const count = el('div', 'ft-count', root);
  const countNum = el('span', null, count);
  el('div', 'ft-grain', root);
  el('div', 'ft-bar ft-top', root);
  el('div', 'ft-bar ft-bottom', root);
  const relief = el('div', 'ft-relief', root);
  const subtitle = el('div', 'ft-subtitle ft-off', root);
  const measure = el('div', 'ft-measure', root);
  const hud = el('div', 'ft-hud', root);
  const timeline = el('div', 'ft-timeline', hud);
  const segments = el('div', 'ft-segments', timeline);
  const played = el('div', 'ft-played', timeline);
  const head = el('div', 'ft-head', el('div', 'ft-headtrack', timeline));
  const controls = el('div', 'ft-controls', hud);
  const play = el('button', 'ft-button', controls);
  const playIcon = el('span', 'ft-icon', play);
  const playLabel = el('span', null, play);
  const time = el('span', 'ft-time', controls);
  el('span', 'ft-grow', controls);
  const hint = el('span', 'ft-hint', controls);
  const leave = el('button', 'ft-button', controls);
  el('span', 'ft-icon', leave, '\uE73F');
  const leaveLabel = el('span', null, leave);

  press(play, () => {
    if (feathertune.controls) feathertune.controls.playPause();
    else document.querySelector('ytmusic-player-bar #play-pause-button')?.click();
    wake();
  });
  press(leave, () => {
    const done = () => document.exitFullscreen?.().catch(() => {});
    if (document.fullscreenElement) return void done();
    document.documentElement.requestFullscreen?.().then(done, () => {});
  });
  timeline.addEventListener('mousedown', (e) => e.preventDefault());
  timeline.addEventListener('click', (e) => {
    const v = video();
    if (!v || !st.v) return;
    const r = timeline.getBoundingClientRect();
    const total = feathertune.clock.duration(v) || st.v.duration / 1000;
    if (r.width > 0 && total > 0) feathertune.clock.seek(v, clamp((e.clientX - r.left) / r.width, 0, 0.999) * total);
    wake();
  });
  root.addEventListener('mousemove', wake);
  root.addEventListener('wheel', (e) => e.preventDefault(), { passive: false });
  document.documentElement.append(root);
  ui = {
    root,
    bgLayers: [bgA, bgB],
    orbs,
    grade,
    lumen,
    camera,
    lines,
    count,
    countNum,
    relief,
    subtitle,
    measure,
    segments,
    played,
    head,
    play,
    playIcon,
    playLabel,
    time,
    hint,
    leaveLabel,
  };
};

const wake = () => {
  if (!ui) return;
  ui.root.classList.remove('ft-idle');
  clearTimeout(st.idle);
  st.idle = setTimeout(() => ui.root.classList.add('ft-idle'), IDLE_MS);
};

// Smooth A/B crossfade of the Gaussian-blurred cover canvas.
const setBackdrop = (url) => {
  const next = url || '';
  if (next === st.bgUrl) return;
  st.bgUrl = next;
  const cur = ui.bgLayers[st.bgSlot];
  const altSlot = 1 - st.bgSlot;
  const alt = ui.bgLayers[altSlot];
  if (!next) {
    cur.classList.remove('ft-show');
    alt.classList.remove('ft-show');
    return;
  }
  alt.style.backgroundImage = `url("${next}")`;
  void alt.offsetWidth;
  alt.classList.add('ft-show');
  cur.classList.remove('ft-show');
  st.bgSlot = altSlot;
};

const setMoodStyle = () => {
  const M = MOODS[st.v.mood];
  ui.root.dataset.mood = st.v.mood;
  ui.root.style.setProperty('--ft-title-face', M.title.face);
  ui.root.style.setProperty('--ft-title-weight', M.title.weight);
  ui.root.style.setProperty('--ft-title-track', M.title.track);
};

const renderTimeline = () => {
  const { sections, texts } = st.v;
  const total = st.v.duration || sections[sections.length - 1].to || 1;
  ui.segments.replaceChildren();
  for (const s of sections) {
    const seg = el('div', `ft-seg ft-k-${s.kind}`, ui.segments);
    seg.title = texts.scenes[s.kind];
    seg.style.left = `${(clamp(s.from / total, 0, 1) * 100).toFixed(3)}%`;
    seg.style.width = `calc(${(clamp((s.to - s.from) / total, 0, 1) * 100).toFixed(3)}% - 2px)`;
  }
};

const renderRelief = () => {
  const { light } = st.v;
  ui.relief.replaceChildren();
  st.relief = null;
  if (light.length < 2) return;
  const W = 1000;
  const H = 100;
  let d = `M0 ${H}`;
  for (let x = 0; x <= W; x += 4) d += ` L${x} ${(H - light[Math.floor((x / W) * (light.length - 1))] * H * 0.92).toFixed(1)}`;
  d += ` L${W} ${H} Z`;
  const shape = (cls, parent) => {
    const svg = svgEl('svg', { viewBox: `0 0 ${W} ${H}`, preserveAspectRatio: 'none', class: cls }, parent);
    svgEl('path', { d }, svg);
    return svg;
  };
  shape('ft-relief-dim', ui.relief);
  const clip = el('div', 'ft-relief-clip', ui.relief);
  const inner = el('div', 'ft-relief-inner', clip);
  shape('ft-relief-lit', inner);
  st.relief = { clip, inner, p: -1 };
};

// ---------- fitting & layout engine ----------

const safeBox = () => ({ x: innerWidth * SAFE.x, y: innerHeight * SAFE.y, w: innerWidth * SAFE.w, h: innerHeight * SAFE.h });

const span = (w, a, b, gap) => {
  let s = 0;
  for (let k = a; k < b; k++) s += w[k];
  return s + gap * Math.max(0, b - a - 1);
};

const balance = (w, R, gap) => {
  const n = w.length;
  R = clamp(R, 1, Math.max(1, n));
  const best = Array.from({ length: R + 1 }, () => new Float64Array(n + 1).fill(Infinity));
  const cut = Array.from({ length: R + 1 }, () => new Int32Array(n + 1));
  best[0][0] = 0;
  for (let r = 1; r <= R; r++) {
    for (let i = r; i <= n; i++) {
      for (let k = r - 1; k < i; k++) {
        const v = Math.max(best[r - 1][k], span(w, k, i, gap));
        if (v < best[r][i]) {
          best[r][i] = v;
          cut[r][i] = k;
        }
      }
    }
  }
  const rows = [];
  for (let r = R, i = n; r > 0; r--) {
    const k = cut[r][i];
    rows.unshift([k, i]);
    i = k;
  }
  return rows;
};

const fit = (w, gap, lh, maxW, maxH, cap, maxRows = 5, minRows = 1) => {
  let best = null;
  const n = w.length;
  for (let R = Math.min(minRows, n); R <= Math.min(maxRows, n); R++) {
    const rows = balance(w, R, gap);
    const widest = Math.max(...rows.map(([a, b]) => span(w, a, b, gap)));
    let size = Math.min(maxH / (R * lh), cap);
    if (widest * size > maxW) size = maxW / widest;
    if (!best || size > best.size * 1.08) best = { rows, size, w };
  }
  return best;
};

const anchor = (box, bw, bh, x, y) => ({
  left: box.x + ((box.w - bw) * (x + 1)) / 2,
  top: box.y + ((box.h - bh) * (y + 1)) / 2,
});

const stackRows = (c, f, align, x, y, box = c.box) => {
  const widths = f.rows.map(([a, b]) => span(f.w, a, b, c.gap) * f.size);
  const bw = Math.max(...widths);
  const bh = f.rows.length * c.lh * f.size;
  const at = anchor(box, bw, bh, x, y);
  return f.rows.map(([a, b], r) => ({
    idx: range(a, b),
    size: f.size,
    width: widths[r],
    left: align === 'c' ? at.left + (bw - widths[r]) / 2 : align === 'r' ? at.left + bw - widths[r] : at.left,
    top: at.top + r * c.lh * f.size,
  }));
};

const echoOf = (words) => {
  const keys = words.map((w) =>
    w.text
      .normalize('NFD')
      .replace(/\p{M}/gu, '')
      .toLowerCase()
      .replace(/[^\p{L}\p{N}]+/gu, ' ')
      .trim(),
  );
  const count = new Map();
  for (const k of keys) if ([...k].length >= 2) count.set(k, (count.get(k) || 0) + 1);
  let best = '';
  for (const [k, n] of count) if (n >= 2 && n <= 4 && n > (count.get(best) || 0)) best = k;
  return best ? keys.flatMap((k, q) => (k === best ? [q] : [])) : null;
};

// 8 curated cinema layouts: balanced, legible, zero gimmicks.
const LAYOUT = {
  focus(c) {
    const f = fit(c.w, c.gap, c.lh, c.box.w * 0.86, c.box.h * 0.78, c.cap, 4);
    return { rows: stackRows(c, f, 'c', c.shot.x, c.shot.y) };
  },
  editorial(c) {
    const wide = c.w.length >= 3 ? 0.68 : 0.86;
    const f = fit(c.w, c.gap, c.lh, c.box.w * wide, c.box.h * 0.8, c.cap, 4, Math.min(2, c.w.length));
    const align = c.shot.x > 0 ? 'r' : c.shot.x < 0 ? 'l' : 'c';
    return { rows: stackRows(c, f, align, c.shot.x, c.shot.y) };
  },
  hero(c) {
    const h = c.hero;
    const before = range(0, h);
    const after = range(h + 1, c.w.length);
    const small = (idx) => idx.map((k) => c.w[k]);
    const hf = fit([c.w[h]], 0, c.lh, c.box.w * 0.92, c.box.h * 0.44, c.box.h * 0.42, 1, 1);
    let sh = hf.size;
    const parts = [before, after].filter((p) => p.length);
    const fits = parts.map((p) => fit(small(p), c.gap, c.slh, c.box.w * 0.82, c.box.h * 0.22, sh * 0.34, 2));
    let s2 = Math.min(sh * 0.34, ...fits.map((f) => f.size));
    const height = () => sh * c.lh + parts.reduce((a, _, k) => a + fits[k].rows.length * c.slh * s2, 0) + parts.length * s2 * 0.38;
    const over = height() / c.box.h;
    if (over > 1) {
      sh /= over;
      s2 /= over;
    }
    const heroW = hf.w[0] * sh;
    const widths = fits.map((f) => f.rows.map(([a, b]) => span(f.w, a, b, c.gap) * s2));
    const bw = Math.max(heroW, ...widths.flat());
    const at = anchor(c.box, bw, height(), c.shot.x, c.shot.y);
    const align = (w) => (c.shot.x > 0 ? at.left + bw - w : c.shot.x < 0 ? at.left : at.left + (bw - w) / 2);
    const rows = [];
    let top = at.top;
    const put = (idx, k) => {
      fits[k].rows.forEach(([a, b], r) => {
        rows.push({ idx: idx.slice(a, b), size: s2, width: widths[k][r], left: align(widths[k][r]), top, small: true });
        top += c.slh * s2;
      });
      top += s2 * 0.38;
    };
    if (before.length) put(before, 0);
    rows.push({ idx: [h], size: sh, width: heroW, left: align(heroW), top });
    top += sh * c.lh + s2 * 0.38;
    if (after.length) put(after, parts.length - 1);
    return { rows };
  },
  split(c) {
    const f = fit(c.w, c.gap, c.lh, c.box.w * 0.8, c.box.h * 0.72, c.cap * 1.02, 2, 2);
    const dir = c.shot.x > 0 ? -1 : 1;
    const widths = f.rows.map(([a, b]) => span(c.w, a, b, c.gap) * f.size);
    const lead = c.lh * f.size * 0.42;
    const bh = 2 * c.lh * f.size + lead;
    const top = anchor(c.box, c.box.w, bh, 0, c.shot.y).top;
    const at = (w, r) => ((r === 0) === dir > 0 ? c.box.x : c.box.x + c.box.w - w);
    const rows = f.rows.map(([a, b], r) => ({
      idx: range(a, b),
      size: f.size,
      width: widths[r],
      left: at(widths[r], r),
      top: top + r * (c.lh * f.size + lead),
      tone: r === 1,
    }));
    return { rows, rule: { top: top + c.lh * f.size + lead / 2, dir } };
  },
  anthem(c) {
    const share = [0.84, 0.88, 0.94, 1.0][c.shot.level];
    const box = {
      x: c.box.x + (c.box.w * (1 - share)) / 2,
      y: c.box.y + (c.box.h * (1 - share)) / 2,
      w: c.box.w * share,
      h: c.box.h * share,
    };
    const cap = c.cap * (1.06 + 0.08 * c.shot.level);
    return { rows: stackRows(c, fit(c.w, c.gap, c.lh, box.w, box.h, cap, 4), 'c', c.shot.x, 0, box) };
  },
  climb(c) {
    const rise = c.box.h * 0.12;
    const box = { x: c.box.x, y: c.box.y + rise / 2, w: c.box.w, h: c.box.h - rise };
    const f = fit(c.w, c.gap, c.lh, box.w * 0.72, box.h * 0.62, c.cap, 2, 2);
    const align = c.shot.x > 0 ? 'r' : c.shot.x < 0 ? 'l' : 'c';
    return { rows: stackRows(c, f, align, c.shot.x, 0, box), rise };
  },
  echo(c) {
    const n = c.w.length;
    const starts = [0, ...c.echo.slice(1)];
    const R = starts.length;
    const f = range(0, R).map((r) => 0.76 + (0.24 * r) / Math.max(1, R - 1));
    const spans = starts.map((a, r) => [a, starts[r + 1] ?? n]);
    const pre = spans.map(([a], r) => (r === 0 && c.echo[0] > a ? span(c.w, a, c.echo[0], c.gap) + c.gap : 0) * f[r]);
    const post = spans.map(([a, b], r) => span(c.w, a, b, c.gap) * f[r] - pre[r]);
    const L = Math.max(...pre);
    const T = L + Math.max(...post);
    const H = c.lh * f.reduce((a, x) => a + x, 0);
    const S = Math.min((c.box.w * 0.88) / T, (c.box.h * 0.82) / H, c.cap * 1.05);
    const at = anchor(c.box, T * S, H * S, c.shot.x, c.shot.y);
    let top = at.top;
    return {
      rows: spans.map(([a, b], r) => {
        const row = {
          idx: range(a, b),
          size: f[r] * S,
          width: span(c.w, a, b, c.gap) * f[r] * S,
          left: at.left + (L - pre[r]) * S,
          top,
          dim: r < R - 1,
        };
        top += c.lh * f[r] * S;
        return row;
      }),
    };
  },
  crescendo(c) {
    const last = c.w.length - 1;
    const hf = fit([c.w[last]], 0, c.lh, c.box.w * 0.94, c.box.h * 0.48, c.box.h * 0.46, 1, 1);
    const f = fit(c.w.slice(0, last), c.gap, c.slh, c.box.w * 0.68, c.box.h * 0.28, hf.size * 0.3, 3);
    let sh = hf.size;
    let s2 = f.size;
    const height = () => f.rows.length * c.slh * s2 + s2 * 0.35 + sh * c.lh;
    const over = height() / c.box.h;
    if (over > 1) {
      sh /= over;
      s2 /= over;
    }
    const heroW = hf.w[0] * sh;
    const widths = f.rows.map(([a, b]) => span(f.w, a, b, c.gap) * s2);
    const bw = Math.max(heroW, ...widths);
    const at = anchor(c.box, bw, height(), c.shot.x, c.shot.y);
    const align = (w) => (c.shot.x > 0 ? at.left + bw - w : c.shot.x < 0 ? at.left : at.left + (bw - w) / 2);
    let top = at.top;
    const rows = f.rows.map(([a, b], r) => {
      const row = { idx: range(a, b), size: s2, width: widths[r], left: align(widths[r]), top, small: true };
      top += c.slh * s2;
      return row;
    });
    rows.push({ idx: [last], size: sh, width: heroW, left: align(heroW), top: top + s2 * 0.35 });
    return { rows };
  },
};

const measure = (items, face, track) => {
  const box = ui.measure;
  box.replaceChildren();
  box.style.fontFamily = face;
  box.style.letterSpacing = `${track}em`;
  const spans = items.map((it) => {
    const s = el('span', it.cls, box, it.text);
    if (it.face) s.style.fontFamily = it.face;
    s.style.fontVariationSettings = it.fv;
    return s;
  });
  const out = spans.map((s) => s.getBoundingClientRect().width / 100);
  box.replaceChildren();
  return out;
};

const bboxOf = (rows, lh) => {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rows) {
    const h = r.size * (r.small ? Math.max(1.08, lh) : lh);
    x0 = Math.min(x0, r.left);
    y0 = Math.min(y0, r.top);
    x1 = Math.max(x1, r.left + r.width);
    y1 = Math.max(y1, r.top + h);
  }
  return { x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) };
};

// ---------- bulletproof album cover & non-text scenes ----------

const attachCoverImg = (container, src) => {
  container.replaceChildren();
  if (!src) return null;
  const img = el('img', 'ft-cover-img', container);
  img.alt = '';
  img.decoding = 'async';
  img.addEventListener('load', () => {
    if (img.naturalWidth > 0 && img.naturalHeight > 0) {
      const ratio = img.naturalWidth / img.naturalHeight;
      // 4:3 YouTube video thumbnails often bake top/bottom black letterbox bars into the JPEG.
      if (src.includes('ytimg.com') && ratio >= 1.25 && ratio <= 1.42) img.classList.add('ft-crop-43');
    }
    img.classList.add('ft-loaded');
  });
  img.addEventListener('error', () => {
    img.remove();
  });
  img.src = src;
  return img;
};

const createCover = (parent, size, withKen = true) => {
  const wrap = el('div', 'ft-cover-wrap', parent);
  wrap.style.width = `${Math.round(size)}px`;
  wrap.style.height = `${Math.round(size)}px`;
  const halo = el('div', 'ft-cover-halo', wrap);
  if (st.v.halo) {
    halo.style.backgroundImage = `url("${st.v.halo}")`;
    halo.classList.add('ft-show');
  }
  const frame = el('div', 'ft-cover-frame', wrap);
  const holder = withKen ? el('div', 'ft-cover-ken', frame) : frame;
  const src = st.v.artwork || fallbackArtwork();
  attachCoverImg(holder, src);
  return { wrap, halo, holder, artUrl: src };
};

// Updates cover/halo/up-next on a live scene in-place when late data arrives for the same track.
const updateSceneInPlace = () => {
  const sc = st.scene;
  if (!sc) return;
  const src = st.v.artwork || fallbackArtwork();
  if (sc.cover && src && src !== sc.cover.artUrl) {
    sc.cover.artUrl = src;
    attachCoverImg(sc.cover.holder, src);
  }
  if (sc.cover && st.v.halo) {
    const bg = `url("${st.v.halo}")`;
    if (sc.cover.halo.style.backgroundImage !== bg) {
      sc.cover.halo.style.backgroundImage = bg;
      sc.cover.halo.classList.add('ft-show');
    }
  }
  if (sc.kind === 'credits' && sc.upNextSlot) {
    renderUpNext(sc.upNextSlot);
  }
};

// Smoothly animated title words inside an overflow-clipped mask (replaces 1ms blinking letters).
const renderMaskedTitle = (parent, str, baseDelay = 160) => {
  const M = MOODS[st.v.mood].title;
  const heading = el('h1', 'ft-title-text', parent);
  const words = cased(str, M.kase).match(/\S+/g) || [];
  if (!words.length) return heading;
  if (lightFx()) {
    heading.textContent = words.join(' ');
    return heading;
  }
  words.forEach((w, idx) => {
    const mask = el('span', 'ft-tw-mask', heading);
    const spanWord = el('span', 'ft-tw', mask, w);
    spanWord.style.setProperty('--d', `${baseDelay + Math.min(idx, 10) * 65}ms`);
    if (idx < words.length - 1) heading.append(' ');
  });
  return heading;
};

const renderUpNext = (slot) => {
  slot.replaceChildren();
  const nx = st.v.credits.next;
  if (!nx || !nx.title) return;
  const card = el('div', 'ft-up-next', slot);
  el('span', 'ft-eyebrow', card, st.v.texts.next);
  const body = el('div', 'ft-up-next-body', card);
  el('b', null, body, nx.title);
  if (nx.artist) el('span', null, body, nx.artist);
};

const track = (a, b) => {
  st.anims.add(a);
  b?.anims.add(a);
  const done = () => {
    st.anims.delete(a);
    b?.anims.delete(a);
  };
  a.finished.then(done, done);
};

const run = (node, frames, dur, since, easing = EASE_FILM, b = null) => {
  const a = node.animate(frames, { duration: dur, easing, fill: 'both' });
  const at = st.now - since;
  if (at >= dur) {
    a.finish();
    return a;
  }
  a.currentTime = Math.max(0, at);
  if (st.still) a.pause();
  track(a, b);
  return a;
};

const placeCount = (x, y, size) => {
  Object.assign(ui.count.style, { left: `${x.toFixed(0)}px`, top: `${y.toFixed(0)}px`, fontSize: `${size.toFixed(0)}px` });
  st.count = { n: 0 };
};
const showCount = (n) => {
  const c = st.count;
  if (!c || c.n === n) return;
  c.n = n;
  ui.count.classList.toggle('ft-show', n > 0);
  ui.count.classList.toggle('ft-last', n === 1);
  if (!n) return;
  ui.countNum.textContent = String(n);
  if (!lightFx()) {
    ui.countNum.animate(
      [
        { transform: 'scale(1.18)', opacity: 0.25 },
        { transform: 'none', opacity: 1 },
      ],
      { duration: 340, easing: EASE_FILM },
    );
  }
};
const hideCount = () => {
  st.count = null;
  ui.count.classList.remove('ft-show', 'ft-last');
};

// Removes any active non-text scene cleanly; guarantees zero overlapping duplicate covers.
const dropScene = (immediate = false) => {
  if (st.outSceneNode) {
    st.outSceneNode.remove();
    st.outSceneNode = null;
  }
  const sc = st.scene;
  const a = st.sceneOut;
  st.scene = null;
  st.sceneOut = null;
  if (!sc) return;
  const node = sc.node;
  for (const anim of [...st.anims]) {
    const target = anim.effect?.target;
    if (target && node.contains(target)) {
      anim.cancel();
      st.anims.delete(anim);
    }
  }
  const gone = () => {
    node.remove();
    if (st.outSceneNode === node) st.outSceneNode = null;
  };
  if (immediate || st.still || a?.playState === 'finished' || a?.playState === 'paused') return gone();
  st.outSceneNode = node;
  if (a) return void a.finished.then(gone, gone);
  node.animate([{ opacity: 1, transform: 'scale(1)' }, { opacity: 0, transform: 'scale(0.97)' }], { duration: 420, easing: EASE_EXIT, fill: 'forwards' }).finished.then(gone, gone);
};

const mount = (k, ms) => {
  const v = st.v;
  const sec = v.sections[k];
  st.sec = k;
  st.j = -1;
  hideCount();
  // Moving directly from one non-text scene to another drops the old cover immediately so 2 never coexist.
  dropScene(!TEXT_KINDS.has(sec.kind));
  ui.root.dataset.scene = sec.kind;
  if (TEXT_KINDS.has(sec.kind)) return;

  showSubtitle('');
  dropAside(ms);
  const b = st.block;
  if (b) startExit(b, b.plan && b.plan.at <= ms ? b.plan : quick(b, ms), ms, null);

  const scene = el('div', `ft-scene ft-s-${sec.kind}`, ui.camera);
  const c = v.credits;
  const W = innerWidth;
  const H = innerHeight;
  let cover = null;
  let upNextSlot = null;

  if (sec.kind === 'opening') {
    const wide = W / H > 1.15;
    const flip = wide && hash(v.id || c.title) % 2 === 1;
    const size = wide ? Math.min(H * 0.44, W * 0.31) : Math.min(H * 0.3, W * 0.56);
    const grid = el('div', `ft-open-grid${flip ? ' ft-flip' : ''}${wide ? '' : ' ft-stack'}`, scene);
    cover = createCover(grid, size, true);
    const meta = el('div', 'ft-open-meta', grid);
    if (c.artist) el('p', 'ft-eyebrow', meta, c.artist);
    renderMaskedTitle(meta, c.title || ' ', 180);
    if (c.album) {
      el('div', 'ft-open-rule', meta);
      el('p', 'ft-meta-sub', meta, c.album);
    }
    placeCount(W * 0.5, H * 0.81, Math.min(H * 0.065, 58));
  } else if (sec.kind === 'instrumental') {
    const size = Math.min(H * 0.46, W * 0.34);
    const grid = el('div', 'ft-inst-grid', scene);
    el('p', 'ft-eyebrow', grid, v.texts.instrumental);
    cover = createCover(grid, size, true);
    if (c.title) el('p', 'ft-inst-track', grid, c.artist ? `${c.title} — ${c.artist}` : c.title);
    placeCount(W * 0.82, H * 0.48, Math.min(H * 0.16, 130));
  } else if (sec.kind === 'credits') {
    const box = el('div', 'ft-credits-box', scene);
    const thumbSize = Math.min(H * 0.17, W * 0.14, 160);
    cover = createCover(box, thumbSize, false);
    el('p', 'ft-eyebrow', box, v.texts.listened);
    renderMaskedTitle(box, c.title || ' ', 120);
    const by = [c.artist, c.album].filter(Boolean).join('  ·  ');
    if (by) el('p', 'ft-meta-sub', box, by);
    upNextSlot = el('div', null, box);
    renderUpNext(upNextSlot);
  }
  st.scene = { node: scene, kind: sec.kind, cover, upNextSlot };
};

const showSubtitle = (s) => {
  if (ui.subtitle.textContent !== s) ui.subtitle.textContent = s;
  ui.subtitle.classList.toggle('ft-off', !s);
};

// ---------- lines coming and going ----------

const freeze = (b) => {
  for (const a of b.anims) {
    try {
      a.commitStyles();
    } catch {}
    a.cancel();
    st.anims.delete(a);
  }
  b.anims.clear();
};

const tuck = (b, x, y, s) => {
  b.node.style.transform = `translate3d(${x.toFixed(0)}px, ${y.toFixed(0)}px, 0) scale(${s.toFixed(3)})`;
};

const stepAside = (b) => {
  freeze(b);
  const box = safeBox();
  const s = 0.36;
  const { x, y } = b.center;
  const tx = b.singer === 0 ? box.x - (x - (s * b.bounds.w) / 2) : box.x + box.w - (x + (s * b.bounds.w) / 2);
  tuck(b, tx, box.y - (y - (s * b.bounds.h) / 2), s);
  b.node.classList.add('ft-aside');
  st.aside = b;
};

const pushUp = (b, by, ms) => {
  freeze(b);
  if (b.pushed) goOut(b.pushed, { kind: 'fade', move: EXIT_MS.fade, fade: HANDOFF_MS, at: ms }, ms);
  b.pushed = null;
  const box = safeBox();
  const band = box.h * PUSH_SHARE;
  const s = Math.min(0.46, (band * 0.78) / b.bounds.h, box.w / b.bounds.w);
  tuck(b, 0, box.y + band * 0.1 - (b.center.y - (s * b.bounds.h) / 2), s);
  b.node.classList.add('ft-pushed');
  by.pushed = b;
};

const planExit = (b, next) => {
  let kind = b.exit;
  if (lightFx()) kind = 'dissolve';
  const gap = next - b.end;
  const move = EXIT_MS[kind] || EXIT_MS.dissolve;
  const room = gap >= move;
  const fade = room ? move : clamp(gap, HANDOFF_MS, move);
  const at = Math.min(next, Math.max(next - fade - (room ? 80 : 0), b.last + SEEN_MS));
  return { kind, move, fade, at };
};

const quick = (b, ms) => ({ ...planExit(b, ms), fade: HANDOFF_MS, at: ms });

const goOut = (b, plan, ms) => {
  const s = b.node.style;
  s.setProperty('--xd', `${Math.round(plan.move)}ms`);
  s.setProperty('--xo', `${Math.round(plan.fade)}ms`);
  s.setProperty('--x0', `${Math.round(plan.at - ms)}ms`);
  s.setProperty('--dx', '0px');
  s.setProperty('--dy', '0px');
  b.out = true;
  b.node.classList.add('ft-out', `ft-out-${plan.kind}`);
  const x = { b, at: plan.at, until: plan.at + plan.fade + 80, frozen: plan.at <= ms };
  if (x.frozen) freeze(b);
  st.leaving.push(x);
};

const startExit = (b, plan, ms) => {
  if (st.block === b) st.block = null;
  goOut(b, plan, ms);
  if (b.pushed) goOut(b.pushed, { kind: 'fade', move: EXIT_MS.fade, fade: plan.fade, at: plan.at }, ms);
  b.pushed = null;
};

const dropAside = (ms) => {
  const a = st.aside;
  st.aside = null;
  if (a) goOut(a, { kind: 'dissolve', move: EXIT_MS.dissolve, fade: EXIT_MS.dissolve, at: ms }, ms);
};

const sweep = (ms) => {
  st.leaving = st.leaving.filter((x) => {
    if (!x.frozen && ms >= x.at) {
      x.frozen = true;
      freeze(x.b);
    }
    if (ms < x.until && ms > x.at - 2500) return true;
    freeze(x.b);
    x.b.node.remove();
    return false;
  });
};

const nextAt = (k, j) => {
  const { sections, lines } = st.v;
  const sec = sections[k];
  if (j + 1 < sec.lines.length) return { ms: lines[sec.lines[j + 1]].ms, shot: sec.shots[j + 1] || DEFAULT_SHOT };
  const nx = sections[k + 1];
  if (!nx) return { ms: Infinity, shot: null };
  if (TEXT_KINDS.has(nx.kind) && nx.lines.length) return { ms: lines[nx.lines[0]].ms, shot: nx.shots[0] || DEFAULT_SHOT };
  return { ms: nx.from, shot: null };
};

const keeps = (b, shot) => !lightFx() && (shot.cont || (st.v.duet && b.singer < 2 && shot.singer < 2 && b.singer !== shot.singer));

// Build a line block off-screen (`ft-staged`) with section-aware typography and zero axis stretching.
const buildLine = (si, j) => {
  const v = st.v;
  const sec = v.sections[si];
  const i = sec.lines[j];
  const shot = sec.shots[j] || DEFAULT_SHOT;
  const light = lightFx();
  const l = v.lines[i];
  if (!l || !l.text.trim()) return null;
  const words = wordsOf(i);
  if (!words.length) return null;

  const n = words.length;
  const hero = shot.hero >= 0 && shot.hero < n ? shot.hero : -1;
  let layout = shot.layout;
  const echo = layout === 'echo' ? echoOf(words) : null;
  const within = (a, b) => n >= a && n <= b;
  if (
    (layout === 'hero' && (hero < 0 || n < 2)) ||
    (layout === 'split' && n < 2) ||
    (layout === 'crescendo' && !within(3, 8)) ||
    (layout === 'climb' && !within(4, 12)) ||
    (layout === 'echo' && !echo)
  ) {
    layout = 'focus';
  }

  const M = MOODS[v.mood];
  const T = M[sec.kind] || M.close;
  const role = (k) => {
    if (layout === 'crescendo') return k === n - 1 ? 'hero' : 'small';
    if (layout === 'hero') return k === hero ? 'hero' : 'small';
    if (layout === 'echo') return echo.includes(k) ? 'key' : 'word';
    return k === hero ? 'key' : 'word';
  };

  // Contrasting editorial italic serif on emphasis words when scripted and Latin.
  const italic = words.map((w, k) => shot.italic && (role(k) === 'hero' || role(k) === 'key') && LATIN.test(w.text));
  const shown = words.map((w, k) => (italic[k] ? w.text : cased(w.text, role(k) === 'hero' ? T.heroCase : T.kase)));
  const enter = light ? 'illuminate' : ENTERS.includes(shot.enter) ? shot.enter : 'illuminate';

  const cls = (k) => `ft-mw${role(k) === 'small' ? ' ft-small' : ''}${italic[k] ? ' ft-italic' : ''}`;
  const fvFor = (k) => (italic[k] ? 'normal' : role(k) === 'small' ? T.smallFv : T.fv);
  const faceFor = (k) => (italic[k] ? "'ft-serif', Georgia, serif" : role(k) === 'small' && T.smallFace ? T.smallFace : T.face);

  const items = words.map((_, k) => ({ text: shown[k], cls: cls(k), fv: fvFor(k), face: faceFor(k) }));
  const w = measure(items, T.face, T.track);

  const full = safeBox();
  const side = v.duet && shot.singer < 2 ? (shot.singer === 0 ? -1 : 1) : 0;
  let box = side ? { ...full, x: side < 0 ? full.x : full.x + full.w * 0.28, w: full.w * 0.72 } : full;
  if (shot.cont && !light) box = { ...box, y: box.y + box.h * PUSH_SHARE, h: box.h * (1 - PUSH_SHARE) };

  const calm = sec.kind === 'close';
  const c = {
    w,
    gap: T.gap,
    lh: T.lh,
    slh: Math.max(1.12, T.lh),
    box,
    cap: box.h * (calm ? 0.21 : sec.kind === 'chorus' ? 0.3 : 0.25),
    shot,
    hero,
    echo,
  };
  const g = LAYOUT[layout](c);

  const node = el(
    'div',
    `ft-block ft-staged ft-l-${layout} ft-in-${enter}${v.duet ? ` ft-s${shot.singer}` : ''}`,
    ui.lines,
  );
  const fitBox = el('div', 'ft-fit', node);
  const plane = el('div', 'ft-plane', fitBox);
  const dir = side || (shot.x !== 0 ? shot.x : 1);
  node.style.setProperty('--ft-dir', String(dir));
  node.dataset.level = String(shot.level);

  const holder = el('div', 'ft-inks', plane);
  const inks = g.rows.flatMap((r) => {
    const row = el('div', `ft-row${r.small ? ' ft-small-row' : ''}${r.tone ? ' ft-tone' : ''}${r.dim ? ' ft-echo-dim' : ''}`, holder);
    row.style.setProperty('--ft-row-face', r.small && T.smallFace ? T.smallFace : T.face);
    row.style.setProperty('--ft-row-lh', String(r.small ? c.slh : T.lh));
    row.style.setProperty('--ft-row-gap', `${T.gap}em`);
    row.style.setProperty('--ft-row-track', `${T.track}em`);
    Object.assign(row.style, { left: `${r.left}px`, top: `${r.top}px`, fontSize: `${r.size}px` });
    return r.idx.map((k) => {
      const word = el('span', cls(k).replace('ft-mw', 'ft-w'), row);
      word.style.fontVariationSettings = fvFor(k);
      if (role(k) === 'key' || role(k) === 'hero') word.classList.add('ft-key');
      if (layout === 'crescendo' && k === n - 1) word.classList.add('ft-big');
      if (shot.singer === 2 && k % 2) word.classList.add('ft-alt');
      if (enter === 'curtain') el('span', 'ft-wi', word, shown[k]);
      else word.textContent = shown[k];
      return [k, word];
    });
  });

  if (g.rule && !light) {
    const rule = el('i', 'ft-rule', plane);
    Object.assign(rule.style, { top: `${g.rule.top}px`, left: `${box.x}px`, width: `${box.w}px` });
  }

  const bounds = bboxOf(g.rows, T.lh);
  const scaleK = Math.min(1, box.w / bounds.w, box.h / bounds.h);
  const cx = bounds.x + bounds.w / 2;
  const cy = bounds.y + bounds.h / 2;
  fitBox.style.transformOrigin = `${cx}px ${cy}px`;
  node.style.transformOrigin = `${cx}px ${cy}px`;
  if (scaleK < 0.999) fitBox.style.transform = `scale(${scaleK.toFixed(4)})`;

  // Background vocals: subtle italic serif row below (or above if needed).
  const bgv = [];
  if (l.bg) {
    const size = clamp(Math.min(...g.rows.map((r) => r.size)) * 0.4, 16, 30);
    const half = (bounds.h * scaleK) / 2;
    const below = cy + half + size * 0.6;
    const top = below + size * 2.2 <= innerHeight * (SAFE.y + SAFE.h + 0.06) ? below : cy - half - size * 1.8;
    const ax = side || shot.x;
    const row = el('div', 'ft-bgv', node);
    Object.assign(row.style, {
      left: `${box.x}px`,
      width: `${box.w}px`,
      top: `${Math.max(0, top)}px`,
      fontSize: `${size}px`,
      textAlign: ax < 0 ? 'left' : ax > 0 ? 'right' : 'center',
    });
    for (const b of l.bg) {
      const t = b.text.trim();
      if (t) bgv.push({ el: el('span', 'ft-w', row, t), ms: b.ms });
    }
  }

  const list = [
    ...inks.map(([k, word]) => ({
      el: word,
      ms: words[k].ms,
      on: false,
      boom: layout === 'crescendo' && k === n - 1,
    })),
    ...bgv.map((b) => ({ el: b.el, ms: b.ms, on: false, boom: false })),
  ];

  return {
    k: si,
    j,
    i,
    shot,
    node,
    plane,
    words: list,
    anims: new Set(),
    exit: shot.exit,
    dir,
    level: shot.level,
    chorus: sec.kind === 'chorus',
    singer: v.duet ? shot.singer : 0,
    bounds: { x: cx - (bounds.w * scaleK) / 2, y: cy - (bounds.h * scaleK) / 2, w: bounds.w * scaleK, h: bounds.h * scaleK },
    center: { x: cx, y: cy },
    ms: l.ms,
    first: words[0].ms,
    last: Math.max(words[n - 1].ms, ...bgv.map((b) => b.ms)),
    end: v.ends[i],
    rise: g.rise && !light ? g.rise : 0,
    plan: null,
    held: false,
    out: false,
    pushed: null,
  };
};

const showLine = (nb, ms) => {
  const v = st.v;
  const prev = st.block;
  const light = lightFx();
  const aside = v.duet && !light && !!prev && prev.singer < 2 && nb.singer < 2 && prev.singer !== nb.singer;
  if (aside || !v.duet || nb.singer === 2 || st.aside?.singer === nb.singer) dropAside(ms);
  if (prev) {
    if (aside) stepAside(prev);
    else if (nb.shot.cont && !light) pushUp(prev, nb, ms);
    else startExit(prev, prev.plan || planExit(prev, nb.first), ms);
  }
  st.block = nb;
  nb.node.classList.remove('ft-staged');
  if (nb.rise) {
    const span = Math.max(1600, nb.end - nb.ms + CLIMB_TAIL_MS);
    run(nb.plane, [{ transform: `translateY(${nb.rise / 2}px)` }, { transform: `translateY(${-nb.rise / 2}px)` }], span, nb.ms, 'linear', nb);
  }
  showSubtitle(v.translation?.[nb.i] || '');
};

const upcoming = () => {
  const { sections } = st.v;
  const sec = sections[st.sec];
  if (!sec) return null;
  if (TEXT_KINDS.has(sec.kind) && st.j + 1 < sec.lines.length) return { k: st.sec, j: st.j + 1 };
  const nx = sections[st.sec + 1];
  return nx && TEXT_KINDS.has(nx.kind) && nx.lines.length ? { k: st.sec + 1, j: 0 } : null;
};

const dropStaged = () => {
  if (st.stageJob) window.cancelIdleCallback ? cancelIdleCallback(st.stageJob) : clearTimeout(st.stageJob);
  st.stageJob = 0;
  st.next?.node.remove();
  st.next = null;
};

const takeStaged = (k, j) => {
  const n = st.next;
  if (n && n.k === k && n.j === j) {
    st.next = null;
    return n;
  }
  dropStaged();
  return null;
};

const stageNext = () => {
  const u = upcoming();
  if (st.next && (!u || st.next.k !== u.k || st.next.j !== u.j)) dropStaged();
  if (!u || st.next || st.stageJob) return;
  const job = () => {
    st.stageJob = 0;
    const now = upcoming();
    if (!st.open || !st.v || st.next || !now || now.k !== u.k || now.j !== u.j) return;
    st.next = buildLine(u.k, u.j);
  };
  st.stageJob = window.requestIdleCallback ? requestIdleCallback(job, { timeout: 300 }) : setTimeout(job, 40);
};

const clearAll = (keepScene = false) => {
  for (const a of [...st.anims]) {
    if (keepScene && st.scene?.node.contains(a.effect?.target)) continue;
    a.cancel();
    st.anims.delete(a);
  }
  dropStaged();
  st.block = null;
  st.aside = null;
  st.leaving = [];
  st.j = -1;
  ui.lines.replaceChildren();
  hideCount();
  if (keepScene) return;
  ui.camera.querySelectorAll('.ft-scene').forEach((s) => s.remove());
  st.scene = null;
  st.sceneOut = null;
  st.outSceneNode = null;
};

const lightWord = (b, w, on, ms) => {
  w.on = on;
  w.el.classList.toggle('ft-on', on);
  if (w.boom) {
    b.node.classList.toggle('ft-boom', on);
    if (on && ms - w.ms < 350 && !lightFx()) st.lumenPeak = Math.max(st.lumenPeak, 0.34);
  }
};

// ---------- atmosphere, camera & frame loop ----------

const energy = (t, playing) => {
  const { light } = st.v;
  if (light.length) return light[clamp(Math.floor(t / 100), 0, light.length - 1)];
  const db = playing ? feathertune.audio?.level?.(true) : null;
  st.live ||= db != null;
  if (num(db)) st.e += (clamp((db - LIVE_DB.FLOOR) / LIVE_DB.RANGE, 0, 1) - st.e) * LIVE_DB.FOLLOW;
  else if (db === -Infinity) st.e += -st.e * LIVE_DB.FOLLOW;
  return st.e;
};

const releaseLevel = () => {
  if (!st.live) return;
  st.live = false;
  feathertune.audio?.level?.(false);
};

const setStyle = (node, key, prop, value) => {
  if (!node || st.shown[key] === value) return;
  st.shown[key] = value;
  node.style[prop] = value;
};

const camPose = (sec, p) => {
  const lvl = sec.kind === 'chorus' ? Math.min(3, sec.chorus) : 0;
  let x = 0;
  let y = 0;
  let s = 1;
  switch (sec.camera) {
    case 'push':
      s = 1 + p * (sec.kind === 'chorus' ? 0.025 + 0.008 * lvl : 0.024);
      break;
    case 'pull':
      s = 1.024 - p * 0.024;
      break;
    case 'left':
    case 'right':
      x = (sec.camera === 'left' ? -1 : 1) * (p - 0.5) * 1.0;
      s = 1.008;
      break;
    case 'rise':
      y = (0.5 - p) * 1.4;
      break;
  }
  return { x, y, s };
};

const camAt = (k, ms) => {
  const { sections, grid } = st.v;
  const prog = (s, at) => clamp((at - s.from) / Math.max(1000, s.to - s.from), 0, 1);
  const sec = sections[k];
  const now = camPose(sec, prog(sec, ms));
  const prev = sections[k - 1];
  const span = grid ? clamp(grid.beat * 4, BLEND_MS.MIN, BLEND_MS.MAX) : BLEND_MS.NO_GRID;
  const x = (ms - sec.from) / span;
  if (!prev || x < 0 || x >= 1) return now;
  const a = camPose(prev, prog(prev, prev.to));
  const m = smooth(x);
  return { x: a.x + (now.x - a.x) * m, y: a.y + (now.y - a.y) * m, s: a.s + (now.s - a.s) * m };
};

const hitAt = (sec) => {
  if (sec.hit != null) return sec.hit;
  const { lines, grid } = st.v;
  const l = lines[sec.lines[0]];
  const first = l ? (l.words?.[0]?.ms ?? l.ms) : sec.from + LEAD_MS;
  let hit = first;
  if (grid) {
    const bar = grid.beat * 4;
    const near = grid.off + Math.round((first - grid.off) / bar) * bar;
    if (Math.abs(near - first) <= grid.beat * 0.75) hit = near;
  }
  sec.hit = hit;
  return hit;
};

const entryFx = (k, t) => {
  const fx = { zoom: 0, hit: 0 };
  if (lightFx()) return fx;
  const { sections, grid } = st.v;
  const bar = grid ? grid.beat * 4 : 2000;
  let wind = 0;
  let hit = 0;
  let dip = 0;
  for (const q of [k, k + 1]) {
    const s = sections[q];
    if (s?.entry === 'lift' && s.lines.length) {
      const h = hitAt(s);
      if (t >= h - bar && t < h) wind = ((t - (h - bar)) / bar) ** 2;
      else if (t >= h && t < h + 1400) hit = Math.exp(-(t - h) / 360);
    } else if (s?.entry === 'breath' && q === k) {
      const x = (t + LEAD_MS - s.from) / bar;
      if (x >= 0 && x < 1) dip = Math.sin(Math.PI * x);
    }
  }
  fx.zoom = 0.025 * wind + 0.01 * hit - 0.018 * dip;
  fx.hit = hit;
  return fx;
};

const sectionIndexAt = (sections, ms) => {
  const k = sections.findIndex((s) => ms >= s.from && ms < s.to);
  return k >= 0 ? k : ms < sections[0].from ? 0 : sections.length - 1;
};

const draw = (v, playing) => {
  const data = st.v;
  const t = feathertune.clock.time(v) * 1000;
  const ms = t + LEAD_MS;
  if (st.resync) {
    st.resync = false;
    clearAll();
    st.sec = -1;
  }
  st.now = ms;
  if (st.leaving.length) sweep(ms);

  const { sections, lines, grid } = data;
  const k = sectionIndexAt(sections, ms);
  if (k !== st.sec) mount(k, ms);
  const sec = sections[k];
  const light = lightFx();

  if (TEXT_KINDS.has(sec.kind) && sec.lines.length) {
    let j = 0;
    sec.lines.forEach((li, jj) => {
      if (lines[li].ms <= ms) j = jj;
    });
    if (j !== st.j) {
      st.j = j;
      const nb = takeStaged(k, j) || buildLine(k, j);
      if (nb) showLine(nb, ms);
      else {
        if (st.block) startExit(st.block, st.block.plan || quick(st.block, ms), ms);
        dropAside(ms);
        showSubtitle('');
      }
    }
    const b = st.block;
    if (b) {
      for (const w of b.words) {
        const on = ms >= w.ms;
        if (on !== w.on) lightWord(b, w, on, ms);
      }
      if (!b.plan && !b.held) {
        const nx = nextAt(k, j);
        if (nx.shot && keeps(b, nx.shot)) b.held = true;
        else b.plan = planExit(b, nx.ms);
      }
      if (b.plan && ms >= b.plan.at) startExit(b, b.plan, ms);
    }
  } else {
    if (st.count) {
      const next = lines.find((l) => l.ms > ms && l.text.trim());
      const left = next ? next.ms - ms : Infinity;
      const step = grid ? grid.beat : 1000;
      showCount(left <= step * 4 ? Math.max(1, Math.ceil(left / step)) : 0);
    }
    const nx = sections[k + 1];
    if (st.scene && !st.sceneOut && nx && TEXT_KINDS.has(nx.kind) && ms >= nx.from - FRAME_OUT_MS) {
      st.sceneOut = run(
        st.scene.node,
        [
          { opacity: 1, transform: 'scale(1)' },
          { opacity: 0, transform: 'scale(0.96)' },
        ],
        FRAME_OUT_MS,
        nx.from - FRAME_OUT_MS,
        EASE_EXIT,
      );
    }
  }

  stageNext();

  // Smooth exponential damping on energy so lighting breathes like film, never jittering.
  const rawE = energy(t, playing);
  st.smoothE += (rawE - st.smoothE) * 0.08;
  const e = st.smoothE;
  st.lumenPeak *= 0.92;
  if (st.lumenPeak < 0.005) st.lumenPeak = 0;

  const fx = entryFx(k, t);
  const chorus = sec.kind === 'chorus';
  const lvl = chorus ? Math.min(3, Math.max(1, sec.chorus)) : 0;
  const barPh = grid ? ((((t - grid.off) / (grid.beat * 4)) % 1) + 1) % 1 : 1;
  const down = grid ? Math.exp(-barPh * 7) : 0;

  const cam = light ? { x: 0, y: 0, s: 1 } : camAt(k, ms);
  const scale = cam.s + fx.zoom;
  setStyle(
    ui.camera,
    'camera',
    'transform',
    `translate3d(${cam.x.toFixed(3)}vw, ${cam.y.toFixed(3)}vh, 0) scale(${scale.toFixed(4)})`,
  );

  // Subtle atmospheric orb breathing + gentle chorus/crescendo lumen.
  const orbOp = clamp((chorus ? 0.56 + lvl * 0.09 : 0.36) + e * 0.24, 0.25, 0.92);
  setStyle(ui.orbs, 'orbs', 'opacity', orbOp.toFixed(2));
  const beatLumen = chorus && grid && !light ? down * (0.14 + 0.05 * lvl) * e : 0;
  const lumenOp = clamp(Math.max(beatLumen, fx.hit * 0.28, st.lumenPeak), 0, 0.42);
  setStyle(ui.lumen, 'lumen', 'opacity', lumenOp.toFixed(2));

  // Subtle cover halo breathing in Opening / Instrumental.
  if (st.scene?.cover?.halo && !light && st.scene.kind !== 'credits') {
    const hs = 1 + e * 0.08;
    setStyle(st.scene.cover.halo, 'haloScale', 'transform', `scale(${hs.toFixed(3)})`);
  }

  // HUD & energy horizon.
  const total = st.total || data.duration / 1000;
  const frac = total > 0 ? clamp(t / 1000 / total, 0, 1) : 0;
  const shown = Math.round(frac * 2000) / 2000;
  if (shown !== st.shown.played) {
    st.shown.played = shown;
    ui.played.style.transform = `scaleX(${shown})`;
    ui.head.parentNode.style.transform = `translateX(${(shown * 100).toFixed(2)}%)`;
  }
  const timeStr = `${fmt(t / 1000)} / ${fmt(total)}`;
  if (timeStr !== st.shown.time) {
    st.shown.time = timeStr;
    ui.time.textContent = timeStr;
  }
  if (st.shown.paused !== v.paused) {
    st.shown.paused = v.paused;
    ui.playIcon.textContent = v.paused ? '\uE768' : '\uE769';
    ui.playLabel.textContent = v.paused ? data.texts.play : data.texts.pause;
  }
  if (st.relief && RELIEF_KINDS.has(sec.kind) && Math.abs(shown - st.relief.p) > 0.0004) {
    st.relief.p = shown;
    st.relief.clip.style.transform = `translateX(${((shown - 1) * 100).toFixed(2)}%)`;
    st.relief.inner.style.transform = `translateX(${((1 - shown) * 100).toFixed(2)}%)`;
  }
};

const running = (v) => st.open && !st.ad && !document.hidden && document.hasFocus() && !!v && !v.paused;

const setStill = (still) => {
  if (still === st.still) return;
  st.still = still;
  ui.root.classList.toggle('ft-still', still);
  for (const a of st.anims) still ? a.pause() : a.play();
};

const frame = () => {
  st.raf = 0;
  if (!st.open || st.ad || !st.v) return;
  if (!st.fonts && performance.now() - st.opened < FONT_WAIT_MS) {
    st.raf = requestAnimationFrame(frame);
    return;
  }
  const v = video();
  const go = running(v);
  setStill(!go);
  if (v) draw(v, go);
  if (go) st.raf = requestAnimationFrame(frame);
  else releaseLevel();
};

const kick = () => {
  if (st.open && !st.ad && st.v && !st.raf) st.raf = requestAnimationFrame(frame);
};

const relayout = () => {
  if (!st.open || !st.v) return;
  st.resync = true;
  kick();
};

const coverSite = () => {
  const on = st.open && !st.ad;
  document.documentElement.toggleAttribute('data-feathertune-stage', on);
  if (feathertune.covered === on) return;
  feathertune.covered = on;
  if (!on) feathertune.audio?.level?.(false);
};

const onAd = (showing) => {
  if (showing === st.ad) return;
  st.ad = showing;
  if (!ui) return;
  ui.root.classList.toggle('ft-ad', showing);
  coverSite();
  if (showing) {
    cancelAnimationFrame(st.raf);
    st.raf = 0;
    releaseLevel();
  } else if (st.open) {
    st.resync = true;
    kick();
  }
};
feathertune.ad?.listen(onAd);

const onResize = () => {
  clearTimeout(st.resize);
  st.resize = setTimeout(relayout, 150);
};

const open = () => {
  st.open = true;
  st.opened = performance.now();
  st.ad = !!feathertune.ad?.showing();
  ui.root.classList.toggle('ft-ad', st.ad);
  ui.root.hidden = false;
  coverSite();
  requestAnimationFrame(() => st.open && ui.root.classList.add('ft-on'));
  addEventListener('resize', onResize);
  wake();
};

const close = () => {
  if (!st.open) return;
  st.open = false;
  cancelAnimationFrame(st.raf);
  st.raf = 0;
  releaseLevel();
  coverSite();
  clearTimeout(st.idle);
  removeEventListener('resize', onResize);
  ui.root.classList.remove('ft-on');
  setTimeout(() => {
    if (st.open) return;
    ui.root.hidden = true;
    clearAll();
    st.sec = -1;
    st.trackId = '';
    st.key = '';
  }, 560);
};

feathertune.stage = Object.freeze({
  set(d) {
    if (d?.open !== true) return close();
    const v = check(d);
    if (!v) {
      console.warn('feathertune: the stage script is not valid');
      return close();
    }
    if (!ui) build();
    const wasOpen = st.open;
    const prevTrack = st.trackId;
    const key = `${v.id}\u0000${v.mood}\u0000${JSON.stringify(v.sections)}`;
    const sameScript = wasOpen && key === st.key;
    const sameTrack = wasOpen && !!v.id && v.id === prevTrack;

    st.v = v;
    st.trackId = v.id;
    st.key = key;
    st.shown = {};
    const playing = video();
    st.total = playing ? feathertune.clock.duration(playing) || 0 : 0;
    if (!st.open) open();

    setMoodStyle();
    setBackdrop(v.backdrop);
    renderTimeline();
    renderRelief();
    ui.hint.textContent = v.texts.esc;
    ui.leaveLabel.textContent = v.texts.leave;

    if (sameScript) {
      dropStaged();
      updateSceneInPlace();
      if (st.block) showSubtitle(v.translation?.[st.block.i] || '');
    } else if (sameTrack && st.scene) {
      // Late lyrics/duration/backdrop update for the SAME track while on a non-text scene:
      // keep the existing scene & cover alive in-place when we are still in the same scene kind!
      const ms = (playing ? feathertune.clock.time(playing) * 1000 : 0) + LEAD_MS;
      const nk = sectionIndexAt(v.sections, ms);
      if (v.sections[nk]?.kind === st.scene.kind) {
        st.sec = nk;
        dropStaged();
        updateSceneInPlace();
      } else {
        clearAll(false);
        st.sec = -1;
      }
    } else {
      clearAll(false);
      st.sec = -1;
    }
    kick();
  },
});

let stageFaces = [];
feathertune.stageFont = Object.freeze({
  set(data) {
    for (const face of stageFaces) document.fonts.delete(face);
    stageFaces = [];
    const list = Array.isArray(data?.fonts) ? data.fonts : [];
    const loads = list
      .filter((f) => ALLOWED_FAMILIES.has(f?.family) && typeof f.data === 'string')
      .map((f) => {
        try {
          const bytes = Uint8Array.from(atob(f.data), (c) => c.charCodeAt(0));
          const desc = {
            style: f.style === 'italic' ? 'italic' : 'normal',
            weight: typeof f.weight === 'string' ? f.weight : '400',
          };
          if (typeof f.stretch === 'string') desc.stretch = f.stretch;
          const face = new FontFace(f.family, bytes.buffer, desc);
          return face.load().then(() => {
            document.fonts.add(face);
            stageFaces.push(face);
          });
        } catch (e) {
          return Promise.reject(e);
        }
      });
    Promise.allSettled(loads).then((all) => {
      for (const r of all) if (r.status === 'rejected') console.warn('feathertune: a stage font did not load', r.reason);
      st.fonts = true;
      // Only re-measure text blocks; never destroy a live opening cover once fonts finish loading.
      if (st.open && st.v && !st.scene) relayout();
      else kick();
    });
  },
});

const onMedia = (e) => {
  if (!st.open || !(e.target instanceof HTMLVideoElement)) return;
  if (e.type !== 'timeupdate' || !st.total) st.total = feathertune.clock.duration(e.target) || 0;
  if (e.type === 'seeked') st.resync = true;
  const ad = !!feathertune.ad?.showing();
  if (ad !== st.ad) onAd(ad);
  kick();
};
for (const type of ['play', 'playing', 'pause', 'seeked', 'timeupdate', 'loadedmetadata']) {
  document.addEventListener(type, onMedia, true);
}
document.addEventListener('visibilitychange', kick);
window.addEventListener('focus', kick);
window.addEventListener('blur', kick);
