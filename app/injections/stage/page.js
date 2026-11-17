// Stage (plugin `stage`, see src/stage.rs): plays the script Rust wrote for the track as a full-screen show,
// in a fixed layer over the site. Rust sends `{ open: true, id, script, lines, translation, field, texts }`
// while the stage is open (again when any of them changes) and `{ open: false }` when it closes (Esc, the
// leave button, hiding or minimizing the window, game mode, the switch); the display fonts come once, as
// `stageFont` `{ fonts: [{ family, data }] }` (the files in base64). The page asks Rust for nothing.
// - Never over an ad: while the core's ad flag is up (`feathertune.ad`) the layer is gone at once (not
//   painted, no mouse) and the site shows; when the ad ends it comes back on the scene of that moment.
// - Under the layer the site's app is `visibility: hidden` (not painted) and `feathertune.covered` is set,
//   which pauses the lyrics' glide and the visualizer.
// - Every line fits on screen: its words are measured once (in a hidden box, with the loaded font, at the
//   axes they end at), then the layout the director chose sets them as big as the safe area (SAFE) allows,
//   breaking into more rows rather than overflowing; titles narrow through the width axis first. The rows
//   as laid out are checked once against what was computed, and the block shrinks if they came out wider.
// - The type: the song's voice (a face and its axes) from the script; a word is not on screen before it is
//   sung. At its time it is born dry (pop: at once, settling from a touch bigger; drift: a short slide
//   from the singer's side; letters: typed over the time it is held), grows to its lit axes while held
//   (font-variation-settings on that word only) and blooms (a copy of the line painted once as glow). The
//   emphasis word gets the singer's color, or the script face. Lines leave (slide, scatter, fold, zoom,
//   sink, steps) as the script says.
// - Pieces from lyric edits: the burst (small words pile up, the last bursts big with a full stop and a
//   flash), the bars (solid, edge to edge, posing a hair either way on the beat while the light paper
//   flickers to the accent), the stair (a diagonal with script ends) and the climb (two rows rising),
//   both with a boiling star (three drawings, a new one every STAR_MS). The paper cuts to light when the
//   script says, with no transition. Flashes and flickers stay under three a second.
// - A duet: each singer on its side in its color, the group in the middle in both; an answer is typed
//   into outlines while the other singer's last line steps aside, small, up in its corner, until that
//   voice returns. Background vocals are a small row under the line, each word born at its own time.
// - One animation frame loop, only while open, the page visible, the track playing and no ad. Paused, it
//   draws one frame and stops; the CSS animations and the axis animations hold too. Per frame only
//   transform and opacity are written (and the star's drawing), only when they change; only the current
//   line (the one leaving, for its exit, and a duet's aside) is in the DOM. With the light effects
//   (`html[data-feathertune-light]`) there is no glow, grain, flash, flicker, pose, boiling, typing, rise,
//   3D or camera breath, and lit words take their axes at once.
// - Without the track analysis the light follows the audio module's live level, read only while the
//   stage plays. Every text goes in as textContent; the data from Rust is checked before use.
if (window.top !== window) return;

// The lyrics' own lead (injections/lyrics/page.js): a line and its words light up this much early.
const LEAD_MS = 250;
const IDLE_MS = 2000;
const KINDS = ['opening', 'close', 'editorial', 'chorus', 'instrumental', 'credits'];
const TEXT_KINDS = new Set(['close', 'editorial', 'chorus']);
const RELIEF_KINDS = new Set(['opening', 'instrumental', 'credits']);
const LAYOUTS = ['stack', 'rag', 'cascade', 'hero', 'grid', 'vertical', 'split', 'tilt', 'anthem', 'burst', 'bar', 'stair', 'climb'];
const ENTERS = ['pop', 'letters', 'drift'];
const EXITS = ['slide', 'scatter', 'fold', 'zoom', 'sink', 'steps'];
const PAPERS = ['dark', 'light'];
const CAMERAS = ['push', 'pull', 'left', 'right', 'rise', 'roll'];
const DEFAULT_SHOT = { layout: 'stack', x: 0, y: 0, hero: -1, enter: 'pop', exit: 'sink', level: 0, paper: 'dark', script: false, singer: 0, answer: false };
// Flashes, flickers and the bars' poses: at most one per this long (three a second).
const FLASH_GAP_MS = 334;
// The star boils: a new drawing this often.
const STAR_MS = 100;
// The star's three drawings: eight uneven points each, in -1..1.
const STARS = [0, 1, 2].map((s) => {
  let d = '';
  for (let p = 0; p < 16; p++) {
    const j = ((p * 7 + s * 13) % 5) / 5;
    const r = p % 2 ? 0.3 + j * 0.14 : 0.74 + j * 0.3;
    const a = (p / 16) * Math.PI * 2 - Math.PI / 2 + (((p * 3 + s * 5) % 7) - 3) * 0.03;
    d += `${p ? 'L' : 'M'}${(Math.cos(a) * r).toFixed(3)} ${(Math.sin(a) * r).toFixed(3)}`;
  }
  return `${d}Z`;
});
// Text the script face can set (Latin, with its accents and punctuation).
const LATIN = /^[ -ɏḀ-ỿ -⁯]+$/;
const MAX_LINES = 2000;
const MAX_LIGHT = 9000;
// Where text may go, as shares of the window: nothing is set outside it (the HUD and the subtitles
// have the space below it).
const SAFE = { x: 0.06, y: 0.1, w: 0.88, h: 0.7 };
// Typed lines only up to this many letters (each letter is an element).
const MAX_LETTERS = 64;
// The live level (dBFS) mapped onto 0..1, and how fast the light follows it.
const LIVE_DB = { FLOOR: -42, RANGE: 34, FOLLOW: 0.15 };
const EASE_IN_OUT = 'cubic-bezier(.76, 0, .24, 1)';
const EASE_OUT = 'cubic-bezier(.16, 1, .3, 1)';
// A word grows to its lit axes over the time it is held, within these.
const HOLD_MS = { MIN: 160, MAX: 2600 };
const EXIT_MS = 700;
const FONT_WAIT_MS = 800;
const FIELD = /^data:image\/png;base64,[A-Za-z0-9+/=]{1,400000}$/;
const TEXTS = {
  leave: 'Leave the stage',
  esc: 'Esc to leave',
  play: 'Play',
  pause: 'Pause',
  chorus: 'Chorus',
  instrumental: 'Instrumental',
  listened: 'You listened to',
  next: 'Up next',
};
// The fonts the page accepts, with their axes' ranges.
const FACES = {
  'ft-anybody': { weight: '100 900', stretch: '50% 150%' },
  'ft-unbounded': { weight: '200 900' },
  'ft-syne': { weight: '400 800' },
  'ft-script': { weight: '400' },
};
// The voices (src/stage.rs, `Voice`): the face, row pitch, word gap and tracking (em), the case, and the
// axes of an unlit word, a lit one, the emphasis word, the small words around a hero; `narrow` is how far
// the width axis may close to fit a title or a hero (none without the axis).
const VOICES = {
  compressed: {
    face: "'ft-anybody'",
    lh: 0.86,
    gap: 0.2,
    track: -0.005,
    kase: 'upper',
    heroCase: 'upper',
    base: [['wdth', 62], ['wght', 640]],
    lit: [['wdth', 74], ['wght', 860]],
    key: [['wdth', 86], ['wght', 900]],
    small: [['wdth', 100], ['wght', 420]],
    smallLit: [['wdth', 104], ['wght', 640]],
    narrow: 50,
  },
  expanded: {
    face: "'ft-anybody'",
    lh: 1.02,
    gap: 0.26,
    track: 0,
    kase: 'asis',
    heroCase: 'upper',
    base: [['wdth', 132], ['wght', 160]],
    lit: [['wdth', 146], ['wght', 380]],
    key: [['wdth', 150], ['wght', 620]],
    small: [['wdth', 118], ['wght', 280]],
    smallLit: [['wdth', 122], ['wght', 420]],
    narrow: 96,
  },
  round: {
    face: "'ft-unbounded'",
    lh: 1.06,
    gap: 0.22,
    track: -0.03,
    kase: 'lower',
    heroCase: 'lower',
    base: [['wght', 240]],
    lit: [['wght', 620]],
    key: [['wght', 880]],
    small: [['wght', 360]],
    smallLit: [['wght', 520]],
    narrow: null,
  },
  quirk: {
    face: "'ft-syne'",
    lh: 1.0,
    gap: 0.24,
    track: -0.02,
    kase: 'asis',
    heroCase: 'upper',
    base: [['wght', 400]],
    lit: [['wght', 700]],
    key: [['wght', 800]],
    small: [['wght', 440]],
    smallLit: [['wght', 620]],
    narrow: null,
  },
};

const st = {
  open: false,
  ad: false,
  raf: 0,
  still: false,
  // The checked data (`check`) and the key of its sections (the same keeps the scene on screen).
  v: null,
  key: '',
  sec: -1,
  line: -2,
  scene: null,
  block: null,
  // A duet's line stepped aside (a block, as `block`).
  aside: null,
  flashAt: 0,
  words: [],
  anims: new Set(),
  dots: null,
  e: 0.5,
  live: false,
  // The display fonts are in (feathertune.stageFont), and when the stage last opened.
  fonts: false,
  opened: 0,
  // The track's length (s), read on the media events rather than at every frame.
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
const video = () => document.querySelector('video');
const lightFx = () => !!feathertune.perf?.light();
const fmt = (s) => {
  const t = Math.max(0, Math.floor(s || 0));
  return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`;
};
const fvs = (axes) => axes.map(([k, v]) => `'${k}' ${Math.round(v * 10) / 10}`).join(', ');
const withWidth = (axes, t) => axes.map(([k, v]) => [k, k === 'wdth' && t != null ? t : v]);
const widthOf = (axes) => axes.find(([k]) => k === 'wdth')?.[1] ?? null;
const cased = (s, kase) => (kase === 'upper' ? s.toLocaleUpperCase() : kase === 'lower' ? s.toLocaleLowerCase() : s);
// A small stable hash, for choices the script does not make (which side the opening's cover is on).
const hash = (s) => [...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);

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

// What Rust sent, checked: anything malformed becomes empty instead of reaching the DOM.
const check = (d) => {
  const s = d?.script;
  if (!s || !num(s.duration_ms) || !Array.isArray(s.sections) || !Array.isArray(d.lines)) return null;
  // [ms, text] pairs: a line's words or its background vocals.
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
    layout: LAYOUTS.includes(x?.layout) ? x.layout : 'stack',
    x: sign(x?.x),
    y: sign(x?.y),
    hero: Number.isInteger(x?.hero) && x.hero >= 0 ? x.hero : -1,
    enter: ENTERS.includes(x?.enter) ? x.enter : 'pop',
    exit: EXITS.includes(x?.exit) ? x.exit : 'sink',
    level: Number.isInteger(x?.level) ? clamp(x.level, 0, 3) : 0,
    paper: PAPERS.includes(x?.paper) ? x.paper : 'dark',
    script: x?.script === true,
    singer: singer(x?.singer),
    answer: x?.answer === true,
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
    duration: s.duration_ms,
    sections,
    voice: Object.hasOwn(VOICES, s.voice) ? s.voice : 'quirk',
    duet: s.duet === true,
    grid,
    light,
    lines,
    ends,
    translation,
    credits: { title: text(c.title, 200), artist: text(c.artist, 200), album: text(c.album, 200), next },
    field: typeof d.field === 'string' && FIELD.test(d.field) ? d.field : null,
    texts,
  };
};

// The words of line `i` with their times; a line timed as a whole gets them spread over its length by
// their letters (as src/stage.rs does, so the emphasis word it chose is the same one).
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

// The cover of the track playing (the media session's largest), when it is a plain https picture.
const artwork = () => {
  const list = navigator.mediaSession?.metadata?.artwork;
  const src = list?.length ? list[list.length - 1].src : '';
  return typeof src === 'string' && /^https:\/\/[^\s"'()]+$/.test(src) ? src : '';
};

const press = (button, f) => {
  button.type = 'button';
  button.tabIndex = -1;
  // The focus stays on the page, so the site's own keys (space, arrows) keep working.
  button.addEventListener('mousedown', (e) => e.preventDefault());
  button.addEventListener('click', f);
};

const build = () => {
  const root = el('div');
  root.id = 'feathertune-stage';
  root.hidden = true;
  const field = el('div', 'ft-field', root);
  const fieldPicture = el('div', 'ft-field-picture', field);
  const lamp2 = el('div', 'ft-lamp ft-lamp2', root);
  const lamp2Core = el('div', 'ft-lamp-core', lamp2);
  const beam = el('div', 'ft-beam', root);
  const beamCore = el('div', 'ft-beam-core', beam);
  const lamp = el('div', 'ft-lamp', root);
  const lampCore = el('div', 'ft-lamp-core', lamp);
  const flash = el('div', 'ft-flash', root);
  const vignette = el('div', 'ft-vignette', root);
  el('div', 'ft-paper', root);
  const pop = el('div', 'ft-pop', root);
  const camera = el('div', 'ft-camera', root);
  const lines = el('div', 'ft-lines', camera);
  const kicker = el('div', 'ft-kicker ft-label', root);
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
  el('span', 'ft-icon', leave, '');
  const leaveLabel = el('span', null, leave);

  press(play, () => {
    if (feathertune.controls) feathertune.controls.playPause();
    else document.querySelector('ytmusic-player-bar #play-pause-button')?.click();
    wake();
  });
  // The page's own fullscreen, entered and left: its end is what tells Rust to close the stage and give
  // the window its size back (src/titlebar.rs), so no new permission is needed.
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
  // The site under it does not scroll.
  root.addEventListener('wheel', (e) => e.preventDefault(), { passive: false });
  document.documentElement.append(root);
  ui = {
    root,
    field,
    fieldPicture,
    lamp,
    lampCore,
    lamp2,
    lamp2Core,
    beam,
    beamCore,
    flash,
    pop,
    camera,
    lines,
    kicker,
    vignette,
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

// The HUD and the cursor go after IDLE_MS without the mouse moving (the subtitles step down into its place).
const wake = () => {
  if (!ui) return;
  ui.root.classList.remove('ft-idle');
  clearTimeout(st.idle);
  st.idle = setTimeout(() => ui.root.classList.add('ft-idle'), IDLE_MS);
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

// The whole song's energy as a relief: a dim one, and the part already played in the accent, shown
// through a clip that slides (two opposite translations, so only transforms move).
const renderRelief = () => {
  const { light } = st.v;
  ui.relief.replaceChildren();
  st.relief = null;
  if (light.length < 2) return;
  const W = 1000;
  const H = 100;
  let d = `M0 ${H}`;
  for (let x = 0; x <= W; x += 4) d += ` L${x} ${(H - light[Math.floor((x / W) * (light.length - 1))] * H * 0.95).toFixed(1)}`;
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

const setField = () => {
  const f = st.v.field;
  ui.fieldPicture.style.backgroundImage = f ? `url("${f}")` : '';
  ui.field.classList.toggle('ft-has-picture', !!f);
};

const setVoice = () => {
  const V = VOICES[st.v.voice];
  ui.root.dataset.voice = st.v.voice;
  ui.root.style.setProperty('--ft-face', V.face);
  ui.root.style.setProperty('--ft-lh', String(V.lh));
  ui.root.style.setProperty('--ft-gap', `${V.gap}em`);
  ui.root.style.setProperty('--ft-track', `${V.track}em`);
};

// ---------- fitting ----------

// The safe area in pixels.
const safeBox = () => ({ x: innerWidth * SAFE.x, y: innerHeight * SAFE.y, w: innerWidth * SAFE.w, h: innerHeight * SAFE.h });

const span = (w, a, b, gap) => {
  let s = 0;
  for (let k = a; k < b; k++) s += w[k];
  return s + gap * Math.max(0, b - a - 1);
};

// Words of widths `w` (em) in `R` rows, in order, the widest row as narrow as it can be: [[from, to), ...].
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

// The rows (between `minRows` and `maxRows`) and the size (px) that set words of widths `w` biggest within
// maxW × maxH, at most `cap`; a row more must earn 6%. With `wn` (the widths at the narrowest width axis)
// a row too wide narrows the axis before the size drops: `t` is the share of the way from narrow (0) to
// lit (1).
const fit = (w, gap, lh, maxW, maxH, cap, maxRows = 6, minRows = 1, wn = null) => {
  let best = null;
  const n = w.length;
  for (let R = Math.min(minRows, n); R <= Math.min(maxRows, n); R++) {
    const rows = balance(w, R, gap);
    const widest = Math.max(...rows.map(([a, b]) => span(w, a, b, gap)));
    let size = Math.min(maxH / (R * lh), cap);
    let t = 1;
    if (widest * size > maxW) {
      const narrowest = wn ? Math.max(...rows.map(([a, b]) => span(wn, a, b, gap))) : widest;
      if (narrowest < widest && narrowest * size <= maxW) t = (maxW / size - narrowest) / (widest - narrowest);
      else if (narrowest < widest) {
        t = 0;
        size = maxW / narrowest;
      } else size = maxW / widest;
    }
    if (!best || size > best.size * 1.06) best = { rows, size, t };
  }
  best.w = wn ? w.map((x, k) => wn[k] + (x - wn[k]) * best.t) : w;
  return best;
};

// Where a block bw × bh sits in `box` for the place (x, y) the script gave (-1, 0 or 1 each way).
const anchor = (box, bw, bh, x, y) => ({ left: box.x + ((box.w - bw) * (x + 1)) / 2, top: box.y + ((box.h - bh) * (y + 1)) / 2 });

// Rows laid out from a fit: each `{ idx, size, left, top, width }`, aligned 'l', 'c' or 'r' in the block.
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

const range = (a, b) => Array.from({ length: Math.max(0, b - a) }, (_, k) => a + k);

// Each layout (src/stage.rs, `Layout`): from the words' widths `c.w` (em) and the shot, the rows in
// pixels, all inside `c.box`. A row may turn (`rot`, degrees about its center) or take the second tone.
const LAYOUT = {
  stack(c) {
    return { rows: stackRows(c, fit(c.w, c.gap, c.lh, c.box.w, c.box.h, c.cap), 'c', c.shot.x, c.shot.y) };
  },
  rag(c) {
    const wide = c.w.length >= 3 ? 0.72 : 1;
    const f = fit(c.w, c.gap, c.lh, c.box.w * wide, c.box.h, c.cap, 5, Math.min(2, c.w.length));
    return { rows: stackRows(c, f, c.shot.x > 0 ? 'r' : 'l', c.shot.x, c.shot.y) };
  },
  cascade(c) {
    const step = c.box.w * 0.07;
    let best = null;
    for (let R = 2; R <= Math.min(5, c.w.length); R++) {
      const rows = balance(c.w, R, c.gap);
      const widest = Math.max(...rows.map(([a, b]) => span(c.w, a, b, c.gap)));
      const size = Math.min((c.box.w - (R - 1) * step) / widest, c.box.h / (R * c.lh), c.cap);
      if (!best || size > best.size * 1.06) best = { rows, size };
    }
    const dir = c.shot.x < 0 ? -1 : 1;
    const widths = best.rows.map(([a, b]) => span(c.w, a, b, c.gap) * best.size);
    const reach = Math.max(...widths.map((w, r) => r * step + w));
    const bh = best.rows.length * c.lh * best.size;
    const at = anchor(c.box, reach, bh, 0, c.shot.y);
    return {
      rows: best.rows.map(([a, b], r) => ({
        idx: range(a, b),
        size: best.size,
        width: widths[r],
        left: dir > 0 ? at.left + r * step : at.left + reach - r * step - widths[r],
        top: at.top + r * c.lh * best.size,
      })),
    };
  },
  hero(c) {
    const h = c.hero;
    const before = range(0, h);
    const after = range(h + 1, c.w.length);
    const small = (idx) => idx.map((k) => c.w[k]);
    // The hero as big as half the height allows, narrowing first; the rest about a third of it.
    const hf = fit([c.w[h]], 0, c.lh, c.box.w, c.box.h * 0.52, c.box.h * 0.5, 1, 1, c.wn ? [c.wn[h]] : null);
    let sh = hf.size;
    const parts = [before, after].filter((p) => p.length);
    const fits = parts.map((p) => fit(small(p), c.gap, c.slh, c.box.w * 0.9, c.box.h * 0.22, sh * 0.3, 2));
    let s2 = Math.min(sh * 0.3, ...fits.map((f) => f.size));
    const height = () => sh * c.lh + parts.reduce((a, _, k) => a + fits[k].rows.length * c.slh * s2, 0) + parts.length * s2 * 0.3;
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
      top += s2 * 0.3;
    };
    if (before.length) put(before, 0);
    rows.push({ idx: [h], size: sh, width: heroW, left: align(heroW), top, t: hf.t });
    top += sh * c.lh + s2 * 0.3;
    if (after.length) put(after, parts.length - 1);
    return { rows };
  },
  grid(c) {
    const n = c.w.length;
    const cols = n <= 4 ? 2 : 3;
    const R = Math.ceil(n / cols);
    const cell = c.box.w / cols;
    const pitch = c.lh * 1.12;
    const size = Math.min(...c.w.map((w) => (cell * 0.94) / w), c.box.h / ((R + 0.35) * pitch), c.cap);
    const gh = (R + 0.35) * pitch * size;
    const top0 = anchor(c.box, c.box.w, gh, 0, c.shot.y).top;
    return {
      rows: c.w.map((w, k) => {
        const col = k % cols;
        const row = Math.floor(k / cols);
        const width = w * size;
        const left = c.box.x + col * cell + (col === 0 ? 0 : col === cols - 1 ? cell - width : (cell - width) / 2);
        return { idx: [k], size, width, left, top: top0 + row * pitch * size + (col % 2) * 0.35 * pitch * size };
      }),
    };
  },
  vertical(c) {
    const h = c.hero;
    // The hero along an edge, reading upward, as long as the height and a quarter of the width allow.
    const hf = fit([c.w[h]], 0, c.lh, c.box.h, c.box.w * 0.26, c.cap * 1.5, 1, 1, c.wn ? [c.wn[h]] : null);
    const sv = hf.size;
    const L = hf.w[0] * sv;
    const T = c.lh * sv;
    const side = c.shot.x > 0 ? 1 : -1;
    const cx = side < 0 ? c.box.x + T / 2 : c.box.x + c.box.w - T / 2;
    const cy = c.box.y + c.box.h / 2;
    const gap = c.box.w * 0.03;
    const rest = range(0, c.w.length).filter((k) => k !== h);
    const area = { x: side < 0 ? c.box.x + T + gap : c.box.x, y: c.box.y, w: c.box.w - T - gap, h: c.box.h };
    const f = fit(
      rest.map((k) => c.w[k]),
      c.gap,
      c.slh,
      area.w,
      area.h,
      Math.min(c.cap, sv * 0.62),
      4,
    );
    const rows = stackRows({ ...c, lh: c.slh }, f, side < 0 ? 'l' : 'r', side, c.shot.y, area).map((r) => ({
      ...r,
      idx: r.idx.map((k) => rest[k]),
      small: true,
    }));
    rows.push({ idx: [h], size: sv, width: L, left: cx - L / 2, top: cy - T / 2, rot: -90, t: hf.t });
    return { rows };
  },
  split(c) {
    const f = fit(c.w, c.gap, c.lh, c.box.w * 0.84, c.box.h / 1.15, c.cap * 1.1, 2, 2);
    const dir = c.shot.x > 0 ? -1 : 1;
    const widths = f.rows.map(([a, b]) => span(c.w, a, b, c.gap) * f.size);
    const lead = c.lh * f.size * 0.3;
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
  tilt(c) {
    // A stack on a plane turned toward the side it sits on; sized so its projection stays in the box.
    const dir = c.shot.x !== 0 ? c.shot.x : c.shot.y >= 0 ? 1 : -1;
    const ry = c.light ? 0 : -22 * dir;
    const rx = c.light ? 0 : 9;
    const rz = c.light ? -2.5 * dir : 0;
    const persp = Math.max(900, innerWidth * 0.75);
    let scale = 0.86;
    let out = null;
    for (let k = 0; k < 3; k++) {
      const box = { x: c.box.x, y: c.box.y, w: c.box.w * scale, h: c.box.h * scale };
      const f = fit(c.w, c.gap, c.lh, box.w, box.h, c.cap);
      const widths = f.rows.map(([a, b]) => span(c.w, a, b, c.gap) * f.size);
      const bw = Math.max(...widths);
      const bh = f.rows.length * c.lh * f.size;
      const p = project(bw, bh, rx, ry, rz, persp);
      out = { f, widths, bw, bh, p };
      const over = Math.max(p.w / c.box.w, p.h / c.box.h);
      if (over <= 1) break;
      scale /= over * 1.02;
    }
    const { f, widths, bw, bh, p } = out;
    // The projection's box placed by the anchor, then the plane's own box from it.
    const at = anchor(c.box, p.w, p.h, c.shot.x, c.shot.y);
    const left = at.left - p.x0 - bw / 2;
    const top = at.top - p.y0 - bh / 2;
    return {
      rows: f.rows.map(([a, b], r) => ({
        idx: range(a, b),
        size: f.size,
        width: widths[r],
        left: left + (bw - widths[r]) / 2,
        top: top + r * c.lh * f.size,
      })),
      plane: { transform: `perspective(${persp}px) rotateX(${rx}deg) rotateY(${ry}deg) rotate(${rz}deg)`, cx: left + bw / 2, cy: top + bh / 2 },
      bounds: { x: at.left, y: at.top, w: p.w, h: p.h },
    };
  },
  anthem(c) {
    // Bigger each time the chorus comes back.
    const share = [0.8, 0.84, 0.92, 1][c.shot.level];
    const box = { x: c.box.x + (c.box.w * (1 - share)) / 2, y: c.box.y + (c.box.h * (1 - share)) / 2, w: c.box.w * share, h: c.box.h * share };
    const cap = c.cap * (1 + 0.12 * c.shot.level);
    return { rows: stackRows(c, fit(c.w, c.gap, c.lh, box.w, box.h, cap, 5), 'c', c.shot.x, 0, box) };
  },
  burst(c) {
    // The last word as big as the box allows (narrowing first), the others piled small above it.
    const last = c.w.length - 1;
    const hf = fit([c.w[last]], 0, c.lh, c.box.w, c.box.h * 0.6, c.box.h * 0.56, 1, 1, c.wn ? [c.wn[last]] : null);
    const f = fit(c.w.slice(0, last), c.gap, c.slh, c.box.w * 0.62, c.box.h * 0.3, hf.size * 0.26, 3);
    let sh = hf.size;
    let s2 = f.size;
    const height = () => f.rows.length * c.slh * s2 + s2 * 0.25 + sh * c.lh;
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
    rows.push({ idx: [last], size: sh, width: heroW, left: align(heroW), top: top + s2 * 0.25, t: hf.t });
    return { rows };
  },
  bar(c) {
    // Centered rows, as big as fits with room for a bar around each; the bars go from edge to edge.
    const pitch = c.lh * 1.36;
    const f = fit(c.w, c.gap, c.lh, c.box.w * 0.92, (c.box.h * c.lh) / pitch, c.cap * 1.2, 3);
    const ph = pitch * f.size;
    const top0 = anchor(c.box, c.box.w, f.rows.length * ph, 0, c.shot.y).top;
    const widths = f.rows.map(([a, b]) => span(f.w, a, b, c.gap) * f.size);
    return {
      rows: f.rows.map(([a, b], r) => ({
        idx: range(a, b),
        size: f.size,
        width: widths[r],
        left: c.box.x + (c.box.w - widths[r]) / 2,
        top: top0 + r * ph + (ph - c.lh * f.size) / 2,
      })),
      bars: f.rows.map((_, r) => ({ top: top0 + r * ph + ph * 0.05, h: ph * 0.9 })),
    };
  },
  stair(c) {
    // One word a row down a diagonal: the ends big (the script face when it can set them), the middle
    // small; the ends' size is the largest that keeps every row inside at its step, and all of them in
    // the height.
    const n = c.w.length;
    const dir = c.shot.x > 0 ? -1 : 1;
    const step = c.box.w * 0.11;
    const f = (k) => (c.big[k] ? 1 : 0.4);
    const pitch = (k) => (c.script[k] ? 0.92 : c.big[k] ? c.lh : c.slh);
    let S = c.box.h * 0.42;
    let h = 0;
    for (let k = 0; k < n; k++) {
      S = Math.min(S, (c.box.w - k * step) / (c.w[k] * f(k)));
      h += f(k) * pitch(k);
    }
    S = Math.min(S, c.box.h / h);
    const sizes = range(0, n).map((k) => S * f(k));
    const widths = sizes.map((s, k) => c.w[k] * s);
    const reach = Math.max(...widths.map((w, k) => k * step + w));
    const at = anchor(c.box, reach, S * h, c.shot.x, c.shot.y);
    let top = at.top;
    const rows = range(0, n).map((k) => {
      const left = dir > 0 ? at.left + k * step : at.left + reach - k * step - widths[k];
      const row = { idx: [k], size: sizes[k], width: widths[k], left, top, lh: pitch(k), small: !c.big[k] };
      top += sizes[k] * pitch(k);
      return row;
    });
    const r0 = rows[0];
    return { rows, stars: [{ x: r0.left + r0.width * (dir > 0 ? 0.14 : 0.86), y: r0.top + r0.size * 0.5, s: r0.size * 1.25 }] };
  },
  climb(c) {
    // Two rows about two thirds of the width, set lower by half the rise so they stay inside as they climb;
    // a star at the first row's start rises with them.
    const rise = c.box.h * 0.16;
    const box = { x: c.box.x, y: c.box.y + rise / 2, w: c.box.w, h: c.box.h - rise };
    const f = fit(c.w, c.gap, c.lh, box.w * 0.64, box.h * 0.6, c.cap, 2, 2);
    const align = c.shot.x > 0 ? 'r' : c.shot.x < 0 ? 'l' : 'c';
    const rows = stackRows(c, f, align, c.shot.x, 0, box);
    const r0 = rows[0];
    const sx = align === 'r' ? r0.left + r0.width + r0.size * 0.1 : r0.left - r0.size * 0.1;
    return { rows, rise, stars: [{ x: sx, y: r0.top + r0.size * 0.45, s: r0.size * 1.5 }] };
  },
};

// The box (relative to the plane's center) a bw × bh plane covers on screen once turned in perspective:
// { w, h, x0, y0 } with x0/y0 the box's top left from the center.
const project = (bw, bh, rx, ry, rz, persp) => {
  const r = Math.PI / 180;
  const pts = [
    [-bw / 2, -bh / 2],
    [bw / 2, -bh / 2],
    [bw / 2, bh / 2],
    [-bw / 2, bh / 2],
  ].map(([x, y]) => {
    // rotate(rz), then rotateY(ry), then rotateX(rx): the order CSS applies them to the element.
    let X = x * Math.cos(rz * r) - y * Math.sin(rz * r);
    let Y = x * Math.sin(rz * r) + y * Math.cos(rz * r);
    let Z = -X * Math.sin(ry * r);
    X *= Math.cos(ry * r);
    const Y2 = Y * Math.cos(rx * r) - Z * Math.sin(rx * r);
    Z = Y * Math.sin(rx * r) + Z * Math.cos(rx * r);
    Y = Y2;
    const k = persp / Math.max(1, persp - Z);
    return [X * k, Y * k];
  });
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const x0 = Math.min(...xs);
  const y0 = Math.min(...ys);
  return { w: Math.max(...xs) - x0, h: Math.max(...ys) - y0, x0, y0 };
};

// The widths (em) of `items` ({ text, cls, fv, letters }) as they will be set, in one layout pass.
const measure = (items) => {
  const box = ui.measure;
  box.replaceChildren();
  const spans = items.map((it) => {
    const s = el('span', it.cls, box);
    s.style.fontVariationSettings = it.fv;
    if (it.letters) for (const ch of it.text) el('span', 'ft-l', s, ch);
    else s.textContent = it.text;
    return s;
  });
  const out = spans.map((s) => s.getBoundingClientRect().width / 100);
  box.replaceChildren();
  return out;
};

// ---------- scenes ----------

const art = (parent) => {
  const src = artwork();
  if (!src) return el('div', 'ft-art ft-art-empty', parent);
  const img = el('img', 'ft-art', parent);
  img.alt = '';
  img.decoding = 'async';
  img.src = src;
  return img;
};

const countdown = (parent) => {
  const box = el('div', 'ft-countdown', parent);
  st.dots = Array.from({ length: 4 }, () => ({ el: el('i', null, box), on: false }));
  return box;
};

// A title (the opening's, the credits') set in `area`: the voice's face at its lit axes, as big as fits,
// narrowing the width axis first, letter by letter rising in from `delay` ms.
const title = (parent, s, area, align, delay) => {
  const V = VOICES[st.v.voice];
  const words = cased(s, V.kase).match(/\S+/g) || [];
  if (!words.length) return null;
  const light = lightFx();
  const narrow = V.narrow != null ? fvs(withWidth(V.key, V.narrow)) : null;
  const items = words.map((t) => ({ text: t, cls: 'ft-mw', fv: fvs(V.key), letters: !light }));
  const w = measure(items);
  const wn = narrow ? measure(items.map((it) => ({ ...it, fv: narrow }))) : null;
  const f = fit(w, V.gap, V.lh, area.w, area.h, area.h * 0.62, 3, 1, wn);
  const fv = V.narrow != null ? fvs(withWidth(V.key, V.narrow + (widthOf(V.key) - V.narrow) * f.t)) : fvs(V.key);
  const c = { gap: V.gap, lh: V.lh };
  const rows = stackRows(c, f, align, align === 'l' ? -1 : align === 'r' ? 1 : 0, 0, area);
  const box = el('div', 'ft-title', parent);
  let n = 0;
  for (const r of rows) {
    const row = el('div', 'ft-row', box);
    Object.assign(row.style, { left: `${r.left}px`, top: `${r.top}px`, fontSize: `${r.size}px` });
    for (const k of r.idx) {
      // The last word of a longer title in the accent.
      const word = el('span', `ft-w ft-on${k === words.length - 1 && k > 0 ? ' ft-key' : ''}`, row);
      word.style.width = `${f.w[k]}em`;
      word.style.fontVariationSettings = fv;
      if (light) word.textContent = words[k];
      else
        for (const ch of words[k]) {
          const l = el('span', 'ft-l', word, ch);
          l.style.animationDelay = `${delay + n++ * 28}ms`;
        }
    }
  }
  return box;
};

// The frame of a scene without lines (opening, instrumental, credits); the text scenes keep the lines'
// layer, whose blocks come and go in `renderLine`.
const mount = (k) => {
  const v = st.v;
  const sec = v.sections[k];
  st.sec = k;
  st.line = -2;
  st.dots = null;
  const lyric = TEXT_KINDS.has(sec.kind);
  if (st.scene) {
    const old = st.scene;
    st.scene = null;
    old.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(1.05)' }], { duration: 600, easing: EASE_IN_OUT, fill: 'forwards' })
      .finished.then(() => old.remove(), () => old.remove());
  }
  ui.root.dataset.scene = sec.kind;
  ui.root.dataset.tone = String(k % 2);
  // The chorus' label, with how many times it came.
  ui.kicker.textContent = sec.kind === 'chorus' ? (sec.chorus > 1 ? `${v.texts.chorus}  ×${sec.chorus}` : v.texts.chorus) : '';
  ui.kicker.classList.toggle('ft-off', sec.kind !== 'chorus');
  showSubtitle('');
  if (lyric) return;
  ui.root.dataset.paper = 'dark';
  dropAside();
  leaveLine();
  // The light back in the middle, behind the frame.
  ui.lamp.style.transform = '';
  ui.lamp2.style.transform = '';
  ui.beam.style.transform = '';
  const scene = el('div', `ft-scene ft-s-${sec.kind}`, ui.camera);
  const c = v.credits;
  const W = innerWidth;
  const H = innerHeight;
  if (sec.kind === 'opening') {
    // A title sequence: the cover on one side, the title set big on the other, then the artist and album.
    const wide = W / H > 1.15;
    const side = hash(v.id || c.title) % 2 ? 1 : -1;
    const size = wide ? Math.min(H * 0.46, W * 0.34) : Math.min(H * 0.32, W * 0.6);
    const cover = art(scene);
    cover.classList.add('ft-cover');
    const box = safeBox();
    let area;
    if (wide) {
      const cx = side < 0 ? box.x + size / 2 : box.x + box.w - size / 2;
      Object.assign(cover.style, { width: `${size}px`, left: `${cx - size / 2}px`, top: `${box.y + (box.h - size) / 2}px` });
      area = { x: side < 0 ? box.x + size + W * 0.05 : box.x, y: box.y + box.h * 0.2, w: box.w - size - W * 0.05, h: box.h * 0.48 };
    } else {
      Object.assign(cover.style, { width: `${size}px`, left: `${(W - size) / 2}px`, top: `${box.y}px` });
      area = { x: box.x, y: box.y + size + H * 0.08, w: box.w, h: Math.max(H * 0.08, box.h - size - H * 0.16) };
    }
    const align = wide ? (side < 0 ? 'l' : 'r') : 'c';
    const t = title(scene, c.title, area, align, 900);
    const label = (s, top, cls) => {
      const p = el('p', `ft-label ${cls}`, scene, s);
      Object.assign(p.style, { top: `${top}px`, left: `${area.x}px`, width: `${area.w}px`, textAlign: { l: 'left', r: 'right', c: 'center' }[align] });
    };
    if (c.artist) label(c.artist, area.y - H * 0.05, 'ft-tracked');
    if (c.album) label(c.album, area.y + area.h + H * 0.03, 'ft-late');
    const dots = countdown(scene);
    Object.assign(dots.style, { top: `${Math.min(area.y + area.h + H * 0.09, H * 0.74)}px`, bottom: 'auto', left: `${area.x + area.w / 2}px` });
    if (!t) cover.classList.add('ft-alone');
    scene.insertAdjacentElement('afterbegin', el('div', 'ft-sweep'));
  } else if (sec.kind === 'instrumental') {
    art(scene);
    el('div', 'ft-label ft-caption ft-tracked', scene, v.texts.instrumental);
    countdown(scene);
  } else if (sec.kind === 'credits') {
    const box = safeBox();
    el('small', 'ft-label ft-credit-label ft-tracked', scene, v.texts.listened).style.top = `${box.y + box.h * 0.04}px`;
    title(scene, c.title, { x: box.x + box.w * 0.08, y: box.y + box.h * 0.14, w: box.w * 0.84, h: box.h * 0.4 }, 'c', 300);
    const by = el('small', 'ft-label ft-credit-by ft-late', scene, [c.artist, c.album].filter(Boolean).join('  ·  '));
    by.style.top = `${box.y + box.h * 0.6}px`;
    if (c.next) {
      const next = el('div', 'ft-up-next', scene);
      el('small', 'ft-label', next, v.texts.next);
      el('b', null, next, c.next.title);
      el('span', null, next, c.next.artist);
    }
  }
  st.scene = scene;
};

const showSubtitle = (s) => {
  if (ui.subtitle.textContent !== s) ui.subtitle.textContent = s;
  ui.subtitle.classList.toggle('ft-off', !s);
};

const onDownbeat = (ms) => {
  const g = st.v.grid;
  if (!g) return false;
  const ph = ((((ms - g.off) / g.beat) % 4) + 4) % 4;
  return ph < 0.25 || ph > 3.75;
};

// The line on screen leaves the way its shot says, and is removed once it has; in a duet it may step
// aside instead: small and dim, up in its singer's corner of the safe area, until that voice comes back.
const leaveLine = (toAside = false) => {
  const b = st.block;
  st.block = null;
  // A word still growing keeps the axes it had reached while it leaves (a climb, its height).
  for (const a of st.anims) {
    try {
      a.commitStyles();
    } catch {}
    a.cancel();
  }
  st.anims.clear();
  st.words = [];
  st.shown.glow = null;
  if (ui) ui.root.dataset.flick = '0';
  if (!b) return;
  const { node, exit, dir } = b;
  if (toAside) {
    const box = safeBox();
    const left = b.singer === 0;
    const ox = left ? b.bounds.x : b.bounds.x + b.bounds.w;
    const oy = b.bounds.y;
    node.style.transformOrigin = `${ox.toFixed(0)}px ${oy.toFixed(0)}px`;
    node.classList.add('ft-aside');
    node.style.transform = `translate(${((left ? box.x : box.x + box.w) - ox).toFixed(0)}px, ${(box.y - oy).toFixed(0)}px) scale(0.34)`;
    st.aside = b;
    return;
  }
  let kind = exit;
  if (lightFx() && (kind === 'fold' || kind === 'scatter')) kind = 'sink';
  node.style.setProperty('--ft-dir', String(dir));
  if (kind === 'scatter') {
    node.querySelectorAll('.ft-inks .ft-w').forEach((w, k) => {
      w.style.setProperty('--sx', `${(((k * 73) % 17) - 8) * 0.9}vw`);
      w.style.setProperty('--sy', `${(((k * 41) % 13) - 6) * 1.4}vh`);
    });
  }
  node.classList.add('ft-out', `ft-out-${kind}`);
  setTimeout(() => node.remove(), EXIT_MS);
};

// The line stepped aside goes, in three steps.
const dropAside = () => {
  const a = st.aside;
  st.aside = null;
  if (!a) return;
  a.node.classList.add('ft-out', 'ft-out-steps');
  setTimeout(() => a.node.remove(), EXIT_MS);
};

// One dry white frame, at most one every FLASH_GAP_MS.
const popFlash = () => {
  const now = performance.now();
  if (lightFx() || now - st.flashAt < FLASH_GAP_MS) return;
  st.flashAt = now;
  ui.pop.classList.remove('ft-go');
  void ui.pop.offsetWidth;
  ui.pop.classList.add('ft-go');
};

// The line `j` of a text scene, set by its shot: measured, fitted, laid out; its words are born as sung.
// `now` is the lyrics' time (ms).
const renderLine = (sec, j, now) => {
  const v = st.v;
  const i = sec.lines[j];
  st.line = i;
  const shot = sec.shots[j] || DEFAULT_SHOT;
  const light = lightFx();
  // A duet: the other singer's line steps aside until that voice comes back; the group takes the stage.
  const prev = st.block;
  const aside = v.duet && !light && !!prev && prev.singer < 2 && shot.singer < 2 && prev.singer !== shot.singer;
  if (aside || !v.duet || shot.singer === 2 || st.aside?.singer === shot.singer) dropAside();
  leaveLine(aside);
  ui.root.dataset.paper = shot.paper;
  const l = v.lines[i];
  if (!l || !l.text.trim()) return showSubtitle('');
  const V = VOICES[v.voice];
  const words = wordsOf(i);
  if (!words.length) return showSubtitle('');
  const n = words.length;
  const hero = shot.hero >= 0 && shot.hero < n ? shot.hero : -1;
  let layout = shot.layout;
  // The script checked the word counts; a line that came out otherwise falls back to a plain stack.
  const within = (a, b) => n >= a && n <= b;
  if (
    ((layout === 'hero' || layout === 'vertical') && (hero < 0 || n < 2)) ||
    (layout === 'grid' && n > 8) ||
    (layout === 'cascade' && n < 3) ||
    (layout === 'burst' && !within(3, 7)) ||
    (layout === 'bar' && n > 8) ||
    (layout === 'stair' && !within(2, 4)) ||
    (layout === 'climb' && !within(4, 12))
  )
    layout = 'stack';
  const big = layout === 'hero' || layout === 'vertical';
  const role = (k) => {
    if (layout === 'burst') return k === n - 1 ? 'hero' : 'small';
    if (layout === 'stair') return k === 0 || k === n - 1 ? 'hero' : 'small';
    return k === hero ? (big ? 'hero' : 'key') : big ? 'small' : 'word';
  };
  // The script face for the hero's word or the stair's ends, when it can set them.
  const script = words.map((w, k) => shot.script && role(k) === 'hero' && (layout === 'stair' || layout === 'hero') && LATIN.test(w.text));
  const shown = words.map((w, k) => (script[k] ? w.text : cased(w.text, role(k) === 'hero' ? V.heroCase : V.kase)));
  // The burst's last word ends the sentence.
  if (layout === 'burst' && !/[.!?…,;:]$/.test(shown[n - 1])) shown[n - 1] += hash(l.text) % 3 ? '.' : '..';
  const letterCount = shown.reduce((a, s) => a + s.length, 0);
  const typed = !light && shot.enter === 'letters' && letterCount <= MAX_LETTERS;
  // A script word keeps its letters joined.
  const lettersOf = (k) => typed && !script[k];
  const enter = typed ? 'letters' : shot.enter === 'drift' && !light ? 'drift' : 'pop';
  const axes = (k) => {
    if (script[k]) return [[], []];
    const r = role(k);
    return r === 'hero' || r === 'key' ? [V.base, V.key] : r === 'small' ? [V.small, V.smallLit] : [V.base, V.lit];
  };
  const cls = (k) => `ft-mw${role(k) === 'small' ? ' ft-small' : ''}${script[k] ? ' ft-script' : ''}`;
  const items = words.map((_, k) => ({ text: shown[k], cls: cls(k), fv: fvs(axes(k)[1]), letters: lettersOf(k) }));
  const w = measure(items);
  // The big words may narrow their width axis to stand bigger.
  let wn = null;
  const narrowable = range(0, n).filter((k) => role(k) === 'hero' && !script[k]);
  if (narrowable.length && V.narrow != null && layout !== 'stair') {
    wn = w.slice();
    const m = measure(narrowable.map((k) => ({ ...items[k], fv: fvs(withWidth(V.key, V.narrow)) })));
    narrowable.forEach((k, q) => (wn[k] = m[q]));
  }
  const full = safeBox();
  // A singer's line keeps to its side, so the other singer's aside has the far corner.
  const side = v.duet && shot.singer < 2 ? (shot.singer === 0 ? -1 : 1) : 0;
  const box = side ? { ...full, x: side < 0 ? full.x : full.x + full.w * 0.3, w: full.w * 0.7 } : full;
  const calm = sec.kind === 'close';
  const c = {
    w,
    wn,
    gap: V.gap,
    lh: V.lh,
    slh: Math.max(1.05, V.lh),
    box,
    cap: box.h * (calm ? 0.3 : 0.4),
    shot,
    hero,
    light,
    big: range(0, n).map((k) => role(k) === 'hero'),
    script,
  };
  const g = LAYOUT[layout](c);
  // The block: the bars and stars behind, a copy of the rows as glow, the rows.
  const ans = shot.answer && typed ? ' ft-ans' : '';
  const node = el('div', `ft-block ft-l-${layout} ft-in-${enter}${ans}${v.duet ? ` ft-s${shot.singer}` : ''}`, ui.lines);
  const fitBox = el('div', 'ft-fit', node);
  const plane = el('div', 'ft-plane', fitBox);
  if (g.plane) {
    plane.style.transform = g.plane.transform;
    plane.style.transformOrigin = `${g.plane.cx}px ${g.plane.cy}px`;
  }
  const dir = side || (shot.x !== 0 ? shot.x : shot.y >= 0 ? 1 : -1);
  node.style.setProperty('--ft-dir', String(dir));
  const ends = (k) => (k + 1 < n ? words[k + 1].ms : v.ends[i]);
  // Born with the voice without axes of their own: the bars (with their row's first word), the stars
  // (with the first word).
  const plain = [];
  const stars = [];
  const deco = g.bars || g.stars ? el('div', 'ft-deco', plane) : null;
  g.bars?.forEach((b, r) => {
    const bar = el('i', 'ft-tbar', deco);
    Object.assign(bar.style, { top: `${b.top}px`, height: `${b.h}px` });
    plain.push([words[g.rows[r].idx[0]].ms, bar]);
  });
  g.stars?.forEach((s, q) => {
    const svg = svgEl('svg', { viewBox: '-1 -1 2 2', class: 'ft-star' }, deco);
    Object.assign(svg.style, { left: `${s.x - s.s / 2}px`, top: `${s.y - s.s / 2}px`, width: `${s.s}px`, height: `${s.s}px` });
    stars.push(svgEl('path', { d: STARS[q % 3] }, svg));
    plain.push([words[g.rows[0].idx[0]].ms, svg]);
  });
  const fvOf = (k, lit) => {
    const r = g.rows.find((row) => row.idx.includes(k));
    const a = axes(k)[lit ? 1 : 0];
    if (r?.t == null || V.narrow == null) return fvs(a);
    // A narrowed hero keeps its width while it lights; only its weight grows.
    const t = V.narrow + (widthOf(V.key) - V.narrow) * r.t;
    return fvs(withWidth(a, t));
  };
  const widthEm = (k) => {
    const r = g.rows.find((row) => row.idx.includes(k));
    return r?.t != null && wn ? wn[k] + (w[k] - wn[k]) * r.t : w[k];
  };
  // One layer of the rows: [[word index, its element], ...].
  const rowsOf = (layer, kind) => {
    const holder = el('div', layer, plane);
    return g.rows.flatMap((r, ri) => {
      const row = el('div', `ft-row${r.small ? ' ft-small-row' : ''}${r.tone ? ' ft-tone' : ''}`, holder);
      Object.assign(row.style, { left: `${r.left}px`, top: `${r.top}px`, fontSize: `${r.size}px`, lineHeight: String(r.lh ?? (r.small ? c.slh : V.lh)) });
      if (r.rot) row.style.transform = `rotate(${r.rot}deg)`;
      row.style.setProperty('--r', String(ri));
      return r.idx.map((k) => {
        const word = el('span', kind === 'ink' ? cls(k).replace('ft-mw', 'ft-w') : `ft-gw${script[k] ? ' ft-script' : ''}`, row);
        word.style.width = `${widthEm(k)}em`;
        if (role(k) === 'key' || role(k) === 'hero') word.classList.add('ft-key');
        if (layout === 'burst' && k === n - 1) word.classList.add('ft-big');
        // A group's words in the two singers' colors, in turn.
        if (shot.singer === 2 && k % 2) word.classList.add('ft-alt');
        if (kind === 'ink') {
          word.style.fontVariationSettings = fvOf(k, false);
          if (enter === 'drift' && shot.singer === 2) word.style.setProperty('--wd', k % 2 ? '1' : '-1');
          if (lettersOf(k)) {
            // Typed over most of the time the word is held.
            const each = clamp(((ends(k) - words[k].ms) * 0.8) / Math.max(1, shown[k].length), 18, 110);
            word.style.setProperty('--lt', `${Math.round(each)}ms`);
            [...shown[k]].forEach((ch, q) => el('span', 'ft-l', word, ch).style.setProperty('--q', String(q)));
          } else word.textContent = shown[k];
        } else {
          word.style.fontVariationSettings = fvOf(k, true);
          word.textContent = shown[k];
        }
        return [k, word];
      });
    });
  };
  const glows = light ? null : rowsOf('ft-glows', 'glow');
  const inks = rowsOf('ft-inks', 'ink');
  if (g.rule && !light) {
    const rule = el('i', 'ft-rule', plane);
    Object.assign(rule.style, { top: `${g.rule.top}px`, left: `${box.x}px`, width: `${box.w}px` });
  }
  // The words as laid out, at their lit axes (the glow copy), against the boxes computed for them: a word
  // that came out wider than its box (a font not loaded yet, rounding) shrinks the block about its center,
  // so nothing leaves the safe area. One layout read for the line.
  const bounds = g.bounds || bboxOf(g.rows);
  let k = Math.min(1, box.w / bounds.w, box.h / bounds.h);
  for (const [, gw] of glows || []) {
    const got = gw.scrollWidth;
    if (got > gw.clientWidth + 1) k = Math.min(k, gw.clientWidth / got);
  }
  const cx = bounds.x + bounds.w / 2;
  const cy = bounds.y + bounds.h / 2;
  fitBox.style.transformOrigin = `${cx}px ${cy}px`;
  node.style.transformOrigin = `${cx}px ${cy}px`;
  if (k < 0.999) fitBox.style.transform = `scale(${k.toFixed(4)})`;
  // The light follows the line.
  ui.lamp.style.transform = `translate(${(cx - innerWidth / 2).toFixed(0)}px, ${(cy - innerHeight / 2).toFixed(0)}px)`;
  ui.lamp2.style.transform = `translate(${((innerWidth / 2 - cx) * 0.8).toFixed(0)}px, ${(innerHeight * 0.18).toFixed(0)}px)`;
  ui.beam.style.transform = `translateX(${(cx - innerWidth / 2).toFixed(0)}px)`;
  node.style.setProperty('--ft-level', String(shot.level));
  node.dataset.level = String(shot.level);
  // The bars pose about the block's center.
  if (layout === 'bar') plane.style.transformOrigin = `${cx}px ${cy}px`;
  // The climb: the rows rise over the line's time (one transform animation, held while paused).
  if (g.rise && !light) {
    const span = Math.max(1500, v.ends[i] - l.ms + EXIT_MS);
    const climb = plane.animate([{ transform: `translateY(${g.rise / 2}px)` }, { transform: `translateY(${-g.rise / 2}px)` }], {
      duration: span,
      easing: 'linear',
      fill: 'both',
    });
    climb.currentTime = clamp(now - l.ms, 0, span);
    if (st.still) climb.pause();
    st.anims.add(climb);
    climb.finished.then(
      () => st.anims.delete(climb),
      () => {},
    );
  }
  // The background vocals: a small row under the line (over it when there is no room), aligned with it.
  const bgv = [];
  if (l.bg) {
    const size = clamp(Math.min(...g.rows.map((r) => r.size)) * 0.34, 15, 34);
    const half = (bounds.h * k) / 2;
    const below = cy + half + size * 0.4;
    const top = below + size * 2.6 <= innerHeight * (SAFE.y + SAFE.h + 0.05) ? below : cy - half - size * 1.6;
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
      if (t) bgv.push({ el: el('span', 'ft-w ft-small', row, cased(t, V.kase)), ms: b.ms });
    }
    for (const b of bgv) b.el.style.fontVariationSettings = fvs(V.small);
  }
  st.words = [
    ...inks.map(([k, word]) => ({
      el: word,
      glow: glows?.find((x) => x[0] === k)?.[1] || null,
      ms: words[k].ms,
      end: ends(k),
      base: fvOf(k, false),
      lit: fvOf(k, true),
      on: false,
      anim: null,
      hit: sec.kind === 'chorus' && onDownbeat(words[k].ms),
      boom: layout === 'burst' && k === n - 1,
      typed: lettersOf(k),
    })),
    ...plain.map(([ms, e]) => ({ el: e, glow: null, ms, end: ms, base: '', lit: '', on: false, anim: null, plain: true })),
    ...bgv.map((b, q) => ({ ...b, glow: null, end: bgv[q + 1]?.ms ?? v.ends[i], base: fvs(V.small), lit: fvs(V.smallLit), on: false, anim: null })),
  ];
  st.block = {
    node,
    plane,
    glows: node.querySelector('.ft-glows'),
    exit: shot.exit,
    dir,
    level: shot.level,
    chorus: sec.kind === 'chorus',
    singer: v.duet ? shot.singer : 0,
    bounds: { x: cx - (bounds.w * k) / 2, y: cy - (bounds.h * k) / 2, w: bounds.w * k, h: bounds.h * k },
    stars,
    pose: -1,
    bar: layout === 'bar' && !light,
    poseIdx: null,
    poseAt: 0,
  };
  showSubtitle(v.translation?.[i] || '');
};

// The box around rows as laid out (a turned row covers its own box turned about its center).
const bboxOf = (rows) => {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  const lh = VOICES[st.v.voice].lh;
  for (const r of rows) {
    const h = r.size * (r.lh ?? (r.small ? Math.max(1.05, lh) : lh));
    let [l, t, w, hh] = [r.left, r.top, r.width, h];
    if (r.rot) [l, t, w, hh] = [r.left + r.width / 2 - h / 2, r.top + h / 2 - r.width / 2, h, r.width];
    x0 = Math.min(x0, l);
    y0 = Math.min(y0, t);
    x1 = Math.max(x1, l + w);
    y1 = Math.max(y1, t + hh);
  }
  return { x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) };
};

// A word starts or stops being sung: it is born, grows to its lit axes over the time it is held (at once
// with the light effects, or when it was already over) and blooms. A typed word lit late starts its
// letters as far along as it is; the burst's last word brings the flash.
const lightWord = (w, on, ms) => {
  w.on = on;
  if (w.typed) w.el.style.setProperty('--l0', `${on ? Math.round(Math.min(0, w.ms - ms)) : 0}ms`);
  w.el.classList.toggle('ft-on', on);
  w.el.classList.toggle('ft-hit', on && !!w.hit);
  w.glow?.classList.toggle('ft-on', on);
  if (w.boom) {
    st.block?.node.classList.toggle('ft-boom', on);
    if (on && ms - w.ms < 400) popFlash();
  }
  if (w.anim) {
    st.anims.delete(w.anim);
    w.anim.cancel();
    w.anim = null;
  }
  if (w.base === w.lit) return;
  if (!on) {
    w.el.style.fontVariationSettings = w.base;
    return;
  }
  const hold = clamp(w.end - w.ms, HOLD_MS.MIN, HOLD_MS.MAX);
  if (lightFx() || ms - w.ms >= hold) {
    w.el.style.fontVariationSettings = w.lit;
    return;
  }
  const a = w.el.animate([{ fontVariationSettings: w.base }, { fontVariationSettings: w.lit }], {
    duration: hold,
    easing: 'cubic-bezier(.3, .1, .2, 1)',
    fill: 'forwards',
  });
  a.currentTime = clamp(ms - w.ms, 0, hold);
  if (st.still) a.pause();
  w.anim = a;
  st.anims.add(a);
  a.finished.then(
    () => {
      st.anims.delete(a);
      if (w.anim === a) {
        w.el.style.fontVariationSettings = w.lit;
        a.cancel();
        w.anim = null;
      }
    },
    () => st.anims.delete(a),
  );
};

// The energy now, 0..1: the analysis' at 10 Hz, else the live level (while the stage plays only).
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

// Writes a style only when it changed.
const setStyle = (node, key, prop, value) => {
  if (!node || st.shown[key] === value) return;
  st.shown[key] = value;
  node.style[prop] = value;
};

// The camera's move over a scene at progress p (0..1), with a breath on the beat.
const cameraAt = (sec, p, breath) => {
  const lvl = sec.kind === 'chorus' ? Math.min(3, sec.chorus) : 0;
  let x = 0;
  let y = 0;
  let s = 1;
  let r = 0;
  switch (sec.camera) {
    case 'push':
      s = 1 + p * (sec.kind === 'chorus' ? 0.03 + 0.012 * lvl : 0.035);
      break;
    case 'pull':
      s = 1.035 - p * 0.035;
      break;
    case 'left':
    case 'right':
      x = (sec.camera === 'left' ? -1 : 1) * (p - 0.5) * 1.6;
      s = 1.012;
      break;
    case 'rise':
      y = (0.5 - p) * 2.2;
      break;
    case 'roll':
      r = (p - 0.5) * 1.4;
      s = 1.02;
      break;
  }
  return `translate(${x.toFixed(3)}vw, ${y.toFixed(3)}vh) rotate(${r.toFixed(3)}deg) scale(${(s + breath).toFixed(4)})`;
};

const draw = (v, playing) => {
  const data = st.v;
  const t = feathertune.clock.time(v) * 1000;
  const ms = t + LEAD_MS;
  const { sections, lines, grid } = data;
  let k = sections.findIndex((s) => ms >= s.from && ms < s.to);
  if (k < 0) k = ms < sections[0].from ? 0 : sections.length - 1;
  if (k !== st.sec) mount(k);
  const sec = sections[k];
  if (TEXT_KINDS.has(sec.kind) && sec.lines.length) {
    let j = 0;
    sec.lines.forEach((li, jj) => {
      if (lines[li].ms <= ms) j = jj;
    });
    if (sec.lines[j] !== st.line) renderLine(sec, j, ms);
    for (const w of st.words) {
      const on = ms >= w.ms;
      if (on !== w.on) lightWord(w, on, ms);
    }
    const b = st.block;
    if (b && !lightFx()) {
      // The star boils: a new drawing every STAR_MS.
      if (b.stars.length) {
        const pose = Math.floor(t / STAR_MS) % 3;
        if (pose !== b.pose) {
          b.pose = pose;
          b.stars.forEach((p, q) => p.setAttribute('d', STARS[(pose + q) % 3]));
        }
      }
      // The bars pose a hair either way on the beat (every other one when beats come faster than
      // FLASH_GAP_MS), the light paper flickering with them; without a tempo, on each word, as often.
      if (b.bar) {
        const lit = st.words.reduce((a, w) => a + (w.on && !w.plain ? 1 : 0), 0);
        const idx = !lit ? null : grid ? Math.floor((t - grid.off) / (grid.beat * (grid.beat < FLASH_GAP_MS ? 2 : 1))) : lit;
        const at = performance.now();
        if (idx !== b.poseIdx && (grid || idx == null || at - b.poseAt >= FLASH_GAP_MS)) {
          b.poseIdx = idx;
          b.poseAt = at;
          const odd = idx != null && (idx & 1) === 1;
          b.plane.style.transform = idx == null ? '' : `rotate(${odd ? 0.75 : -0.75}deg) scale(${odd ? 1.025 : 1})`;
          ui.root.dataset.flick = odd ? '1' : '0';
        }
      }
    }
  } else if (st.dots) {
    // The dots fill over the last four beats (or seconds) before the next sung line.
    const next = lines.find((l) => l.ms > ms && l.text.trim());
    const left = next ? next.ms - ms : Infinity;
    const step = grid ? grid.beat : 1000;
    st.dots.forEach((d, j) => {
      const on = left < step * (4 - j);
      if (on !== d.on) {
        d.on = on;
        d.el.classList.toggle('ft-on', on);
      }
    });
  }
  const e = energy(t, playing);
  const light = lightFx();
  const chorus = sec.kind === 'chorus';
  const lvl = chorus ? Math.min(3, Math.max(1, sec.chorus)) : 0;
  const p = clamp((ms - sec.from) / Math.max(1000, sec.to - sec.from), 0, 1);
  const beatPh = grid ? ((((t - grid.off) / grid.beat) % 1) + 1) % 1 : 0.5;
  const barPh = grid ? ((((t - grid.off) / (grid.beat * 4)) % 1) + 1) % 1 : 1;
  const beat = grid ? Math.exp(-beatPh * 6) : 0;
  const down = grid ? Math.exp(-barPh * 9) : 0;
  setStyle(ui.camera, 'camera', 'transform', cameraAt(sec, p, grid && !light ? beat * 0.005 * e : 0));
  // The light: the lamp behind the line breathes with the energy and kicks on the beat; the second color
  // and the beam come up with it; the vignette opens.
  const q = (x) => x.toFixed(2);
  setStyle(ui.lampCore, 'lamp', 'opacity', q(clamp(0.35 + e * 0.55 + beat * 0.15 * e + lvl * 0.06, 0, 1)));
  setStyle(ui.lampCore, 'lampScale', 'transform', `scale(${(0.9 + e * 0.2 + beat * 0.05 * e).toFixed(3)})`);
  setStyle(ui.lamp2Core, 'lamp2', 'opacity', q(0.25 + e * 0.5));
  setStyle(ui.beamCore, 'beam', 'opacity', q(clamp((chorus ? 0.35 + lvl * 0.15 : 0.12) + e * 0.35, 0, 1)));
  setStyle(ui.vignette, 'vignette', 'opacity', q(0.95 - e * 0.45));
  setStyle(ui.flash, 'flash', 'opacity', chorus && grid && !light ? q(down * (0.45 + 0.15 * lvl) * e) : '0');
  // The glow over the lit words: stronger in the chorus, more each time; it swells on the beat.
  if (st.block?.glows) {
    const base = st.block.chorus ? 0.62 + 0.12 * st.block.level : 0.5;
    setStyle(st.block.glows, 'glow', 'opacity', q(clamp(base * (0.75 + e * 0.35) + (grid ? (chorus ? down * 0.35 : beat * 0.12) : 0), 0, 1)));
  }
  // The HUD and the relief.
  const total = st.total || data.duration / 1000;
  const frac = total > 0 ? clamp(t / 1000 / total, 0, 1) : 0;
  const shown = Math.round(frac * 2000) / 2000;
  if (shown !== st.shown.played) {
    st.shown.played = shown;
    ui.played.style.transform = `scaleX(${shown})`;
    ui.head.parentNode.style.transform = `translateX(${(shown * 100).toFixed(2)}%)`;
  }
  const time = `${fmt(t / 1000)} / ${fmt(total)}`;
  if (time !== st.shown.time) {
    st.shown.time = time;
    ui.time.textContent = time;
  }
  if (st.shown.paused !== v.paused) {
    st.shown.paused = v.paused;
    ui.playIcon.textContent = v.paused ? '' : '';
    ui.playLabel.textContent = v.paused ? data.texts.play : data.texts.pause;
  }
  if (st.relief && RELIEF_KINDS.has(sec.kind) && Math.abs(shown - st.relief.p) > 0.0004) {
    st.relief.p = shown;
    st.relief.clip.style.transform = `translateX(${((shown - 1) * 100).toFixed(2)}%)`;
    st.relief.inner.style.transform = `translateX(${((1 - shown) * 100).toFixed(2)}%)`;
  }
};

const running = (v) => st.open && !st.ad && !document.hidden && !!v && !v.paused;

// Paused: the CSS animations hold (`ft-still`) and so do the words' axis animations.
const setStill = (still) => {
  if (still === st.still) return;
  st.still = still;
  ui.root.classList.toggle('ft-still', still);
  for (const a of st.anims) still ? a.pause() : a.play();
};

const frame = () => {
  st.raf = 0;
  if (!st.open || st.ad || !st.v) return;
  // The first time, the fonts get a moment to load, so the lines are not measured in another face.
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

// Draws now and goes on while it plays; paused, one frame.
const kick = () => {
  if (st.open && !st.ad && st.v && !st.raf) st.raf = requestAnimationFrame(frame);
};

// Everything set again for the new size (the safe area is in pixels).
const relayout = () => {
  if (!st.open || !st.v) return;
  dropAside();
  st.sec = -1;
  kick();
};

// The site shows (and its loops run) whenever the stage is closed or an ad plays.
const cover = () => {
  const on = st.open && !st.ad;
  document.documentElement.toggleAttribute('data-feathertune-stage', on);
  if (feathertune.covered === on) return;
  feathertune.covered = on;
  // Lets the visualizer start again once the site shows.
  if (!on) feathertune.audio?.level?.(false);
};

const onAd = (showing) => {
  if (showing === st.ad) return;
  st.ad = showing;
  if (!ui) return;
  ui.root.classList.toggle('ft-ad', showing);
  cover();
  if (showing) {
    cancelAnimationFrame(st.raf);
    st.raf = 0;
    releaseLevel();
  } else if (st.open) {
    // Back on the scene of this moment.
    st.sec = -1;
    kick();
  }
};
feathertune.ad?.listen(onAd);

// The small cover in the player bar flies to the middle while the stage fades in.
const fly = () => {
  const src = artwork();
  const thumb = [...document.querySelectorAll('ytmusic-player-bar img')].find((i) => i.getBoundingClientRect().width > 0);
  const from = thumb?.getBoundingClientRect();
  if (!src || !from || lightFx()) return;
  const size = Math.min(innerHeight * 0.44, innerWidth * 0.6);
  const img = el('img', 'ft-fly', document.documentElement);
  img.alt = '';
  img.src = src;
  Object.assign(img.style, { left: `${from.left}px`, top: `${from.top}px`, width: `${from.width}px`, height: `${from.height}px` });
  const dx = innerWidth / 2 - size / 2 - from.left;
  const dy = innerHeight / 2 - size / 2 - from.top;
  img
    .animate([{ transform: 'none' }, { transform: `translate(${dx}px, ${dy}px) scale(${size / from.width})` }], {
      duration: 900,
      easing: EASE_IN_OUT,
      fill: 'forwards',
    })
    .finished.then(() => img.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 500, fill: 'forwards' }).finished)
    .catch(() => {})
    .finally(() => img.remove());
};

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
  if (!st.ad) fly();
  cover();
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
  cover();
  clearTimeout(st.idle);
  removeEventListener('resize', onResize);
  ui.root.classList.remove('ft-on');
  setTimeout(() => {
    if (st.open) return;
    ui.root.hidden = true;
    leaveLine();
    st.aside = null;
    ui.lines.replaceChildren();
    ui.root.dataset.paper = 'dark';
    ui.camera.querySelectorAll('.ft-scene').forEach((s) => s.remove());
    st.scene = null;
    st.sec = -1;
  }, 600);
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
    const key = `${v.id}\u0000${v.voice}\u0000${JSON.stringify(v.sections)}`;
    const same = st.open && key === st.key;
    st.v = v;
    st.key = key;
    st.shown = {};
    const playing = video();
    st.total = playing ? feathertune.clock.duration(playing) || 0 : 0;
    if (!st.open) open();
    setVoice();
    renderTimeline();
    renderRelief();
    setField();
    // The same script keeps its scene on screen (the line is set again: a translation may have come).
    if (same) st.line = -2;
    else st.sec = -1;
    ui.hint.textContent = v.texts.esc;
    ui.leaveLabel.textContent = v.texts.leave;
    kick();
  },
});

// The display fonts (Anybody, Unbounded, Syne; SIL Open Font License), loaded with the FontFace API so the
// page fetches nothing. Once they are in, what is on screen is measured and set again with them.
feathertune.stageFont = Object.freeze({
  set(data) {
    const list = Array.isArray(data?.fonts) ? data.fonts : [];
    const loads = list
      .filter((f) => Object.hasOwn(FACES, f?.family) && typeof f.data === 'string')
      .map((f) => {
        try {
          const bytes = Uint8Array.from(atob(f.data), (c) => c.charCodeAt(0));
          const face = new FontFace(f.family, bytes.buffer, FACES[f.family]);
          return face.load().then(() => document.fonts.add(face));
        } catch (e) {
          return Promise.reject(e);
        }
      });
    Promise.allSettled(loads).then((all) => {
      for (const r of all) if (r.status === 'rejected') console.warn('feathertune: a stage font did not load', r.reason);
      st.fonts = true;
      relayout();
    });
  },
});

// The loop starts again on play, after a seek (one frame while paused) and when the page shows again; the
// ad flag is checked on every media event too.
const onMedia = (e) => {
  if (!st.open || !(e.target instanceof HTMLVideoElement)) return;
  if (e.type !== 'timeupdate' || !st.total) st.total = feathertune.clock.duration(e.target) || 0;
  const ad = !!feathertune.ad?.showing();
  if (ad !== st.ad) onAd(ad);
  kick();
};
for (const type of ['play', 'playing', 'pause', 'seeked', 'timeupdate', 'loadedmetadata']) {
  document.addEventListener(type, onMedia, true);
}
document.addEventListener('visibilitychange', kick);
