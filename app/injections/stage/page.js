// Stage (plugin `stage`, see src/stage.rs): plays the script Rust wrote for the track as a full-screen show,
// in a fixed layer over the site. Rust sends `{ open: true, id, script, lines, translation, field, texts }`
// while the stage is open (again when any of them changes) and `{ open: false }` when it closes (Esc, the
// leave button, hiding or minimizing the window, game mode, the switch); the display font comes once, as
// `stageFont` `{ data }` (the file in base64). The page asks Rust for nothing.
// - Never over an ad: while the core's ad flag is up (`feathertune.ad`) the layer is gone at once (not
//   painted, no mouse) and the site shows; when the ad ends it comes back on the scene of that moment.
// - Under the layer the site's app is `visibility: hidden` (not painted) and `feathertune.covered` is set,
//   which pauses the lyrics' glide and the visualizer.
// - One animation frame loop, only while open, the page visible, the track playing and no ad. Paused, it
//   draws one frame and stops; the CSS animations hold too (`ft-still`). Only transform and opacity move;
//   only the current line (and the close-up's next one) is in the DOM. With the light effects
//   (`html[data-feathertune-light]`) there is no grain, echo, flash or camera breath.
// - Without the track analysis the light follows the audio module's live level, read only while the
//   stage plays. Every text goes in as textContent; the data from Rust is checked before use.
if (window.top !== window) return;

// The lyrics' own lead (injections/lyrics/page.js): a line and its words light up this much early.
const LEAD_MS = 250;
const IDLE_MS = 2000;
const KINDS = ['opening', 'close', 'editorial', 'chorus', 'instrumental', 'credits'];
const TEXT_KINDS = new Set(['close', 'editorial', 'chorus']);
const RELIEF_KINDS = new Set(['opening', 'instrumental', 'credits']);
const MAX_LINES = 2000;
const MAX_LIGHT = 9000;
// The live level (dBFS) mapped onto 0..1, and how fast the light follows it.
const LIVE_DB = { FLOOR: -42, RANGE: 34, FOLLOW: 0.15 };
const EASE_IN_OUT = 'cubic-bezier(.7, 0, .2, 1)';
const EASE_OUT = 'cubic-bezier(.2, .7, .2, 1)';
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

const st = {
  open: false,
  ad: false,
  raf: 0,
  // The checked data (`check`) and the key of its sections (the same keeps the scene on screen).
  v: null,
  key: '',
  sec: -1,
  line: -2,
  scene: null,
  words: [],
  dots: null,
  e: 0.5,
  live: false,
  // The track's length (s), read on the media events rather than at every frame.
  total: 0,
  relief: null,
  idle: 0,
  shown: { time: '', paused: null, transform: '', vignette: '', flash: '', played: -1 },
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
  const lines = d.lines.slice(0, MAX_LINES).map((l) => {
    if (!Array.isArray(l)) return { ms: 0, text: '', words: null };
    const words = Array.isArray(l[2])
      ? l[2]
          .filter((w) => Array.isArray(w) && num(w[0]) && typeof w[1] === 'string')
          .slice(0, 200)
          .map(([ms, t]) => ({ ms, text: t.slice(0, 120) }))
      : null;
    return { ms: num(l[0]) ? l[0] : 0, text: text(l[1]), words: words?.length ? words : null };
  });
  const n = lines.length;
  const sections = s.sections
    .filter((x) => x && KINDS.includes(x.kind) && num(x.from_ms) && num(x.to_ms))
    .slice(0, 400)
    .map((x) => ({
      kind: x.kind,
      from: x.from_ms,
      to: x.to_ms,
      lines: Array.isArray(x.lines) ? x.lines.filter((i) => Number.isInteger(i) && i >= 0 && i < n) : [],
      right: x.align === 'right',
      chorus: Number.isInteger(x.chorus_no) ? clamp(x.chorus_no, 0, 99) : 0,
    }));
  if (!sections.length) return null;
  const grid = s.grid && num(s.grid.bpm) && s.grid.bpm > 0 && num(s.grid.offset_ms) ? { beat: 60000 / s.grid.bpm, off: s.grid.offset_ms } : null;
  const light = Float32Array.from(Array.isArray(s.light) ? s.light.slice(0, MAX_LIGHT) : [], (x) => (num(x) ? clamp(x, 0, 255) / 255 : 0));
  const perLine = (list, f) => lines.map((_, i) => f(Array.isArray(list) ? list[i] : undefined, i));
  const ends = perLine(s.ends, (x, i) => (num(x) && x > lines[i].ms ? x : (lines[i + 1]?.ms ?? lines[i].ms + 4000)));
  const echoes = perLine(s.echoes, (x) => (Number.isInteger(x) ? clamp(x, 0, 99) : 0));
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
    grid,
    light,
    lines,
    ends,
    echoes,
    translation,
    credits: { title: text(c.title, 200), artist: text(c.artist, 200), album: text(c.album, 200), next },
    field: typeof d.field === 'string' && FIELD.test(d.field) ? d.field : null,
    texts,
  };
};

// The words of line `i` with their times; a line timed as a whole gets them spread over its length by
// their letters.
const wordsOf = (i) => {
  const { lines, ends } = st.v;
  const l = lines[i];
  if (l.words) return l.words;
  const parts = l.text.match(/\S+\s*/g) || [];
  const span = Math.max(300, ends[i] - l.ms) * 0.85;
  const total = parts.reduce((a, p) => a + p.trim().length, 0) || 1;
  let at = l.ms;
  return parts.map((p) => {
    const w = { ms: Math.round(at), text: p };
    at += (span * p.trim().length) / total;
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
  const flash = el('div', 'ft-flash', root);
  const camera = el('div', 'ft-camera', root);
  const vignette = el('div', 'ft-vignette', root);
  el('div', 'ft-grain', root);
  el('div', 'ft-bar ft-top', root);
  el('div', 'ft-bar ft-bottom', root);
  const relief = el('div', 'ft-relief', root);
  const subtitle = el('div', 'ft-subtitle ft-off', root);
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
  el('span', 'ft-icon', leave, '');
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
  ui = { root, field, fieldPicture, flash, camera, vignette, relief, subtitle, segments, played, head, play, playIcon, playLabel, time, hint, leaveLabel };
};

// The HUD and the cursor go after IDLE_MS without the mouse moving.
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
};

// A new scene: the section's frame (title, cover, credits); the text scenes get their lines in `renderLine`.
const mount = (k) => {
  const v = st.v;
  const sec = v.sections[k];
  st.sec = k;
  st.line = -2;
  st.words = [];
  st.dots = null;
  for (const old of [...ui.camera.children]) if (old !== st.scene) old.remove();
  if (st.scene) {
    const old = st.scene;
    old.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'scale(1.06)' }], { duration: 450, easing: EASE_OUT, fill: 'forwards' })
      .finished.then(() => old.remove(), () => old.remove());
  }
  const scene = el('div', `ft-scene ft-s-${sec.kind}${sec.right ? ' ft-right' : ''}`, ui.camera);
  const c = v.credits;
  if (sec.kind === 'opening') {
    art(scene);
    const title = el('div', 'ft-title ft-reveal', scene);
    const h = el('h1', null, title);
    const words = (c.title || '').split(/(\s+)/);
    const last = words.length > 2 ? words.pop() : '';
    h.textContent = words.join('');
    if (last) el('span', 'ft-accent', h, last);
    el('p', 'ft-label', title, [c.artist, c.album].filter(Boolean).join('  ·  '));
    countdown(scene);
  } else if (sec.kind === 'instrumental') {
    art(scene);
    el('div', 'ft-label ft-caption', scene, v.texts.instrumental);
    countdown(scene);
  } else if (sec.kind === 'credits') {
    const roll = el('div', 'ft-roll ft-reveal', scene);
    el('small', 'ft-label', roll, v.texts.listened);
    el('h2', null, roll, c.title);
    el('small', 'ft-label', roll, [c.artist, c.album].filter(Boolean).join('  ·  '));
    if (c.next) {
      const next = el('div', 'ft-up-next', scene);
      el('small', 'ft-label', next, v.texts.next);
      el('b', null, next, c.next.title);
      el('span', null, next, c.next.artist);
    }
  }
  scene.animate([{ opacity: 0, transform: 'scale(.96)' }, { opacity: 1, transform: 'none' }], { duration: 600, easing: EASE_OUT });
  st.scene = scene;
  ui.root.dataset.scene = sec.kind;
  if (!TEXT_KINDS.has(sec.kind)) showSubtitle('');
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

// The line `i` of a text scene (or nothing: -1), word by word.
const renderLine = (sec, i) => {
  const v = st.v;
  st.line = i;
  st.words = [];
  const scene = st.scene;
  scene.replaceChildren();
  const l = v.lines[i];
  if (!l || !l.text.trim()) return showSubtitle('');
  const words = wordsOf(i);
  const spans = [];
  const put = (parent, w) => {
    const span = el('span', 'ft-w', parent, w.text);
    if (sec.kind === 'chorus' && onDownbeat(w.ms)) span.classList.add('ft-hit');
    spans.push(span);
  };
  const len = l.text.length;
  if (sec.kind === 'close') {
    const line = el('div', 'ft-line', scene);
    line.style.setProperty('--ft-len', String(len));
    for (const w of words) put(line, w);
    const next = sec.lines.find((j) => j > i && v.lines[j].text.trim());
    if (next != null) el('div', 'ft-next', scene, v.lines[next].text);
  } else if (sec.kind === 'editorial') {
    el('div', 'ft-index ft-label', scene, `${String(i + 1).padStart(2, '0')} / ${String(v.lines.length).padStart(2, '0')}`);
    const stack = el('div', 'ft-stack', scene);
    const rows = words.length > 8 ? 4 : words.length > 5 ? 3 : words.length > 1 ? 2 : 1;
    const per = Math.ceil(words.length / rows);
    let chars = 1;
    for (let r = 0; r < rows; r++) {
      const part = words.slice(r * per, (r + 1) * per);
      if (!part.length) continue;
      chars = Math.max(chars, part.reduce((a, w) => a + w.text.length, 0));
      const row = el('span', 'ft-row', stack);
      for (const w of part) put(row, w);
    }
    stack.style.setProperty('--ft-chars', String(chars));
  } else {
    el('div', 'ft-tag ft-label', scene, sec.chorus > 1 ? `${v.texts.chorus}  ×${sec.chorus}` : v.texts.chorus);
    if (!lightFx()) {
      for (let g = 0; g < Math.min(2, v.echoes[i]); g++) el('div', `ft-ghost${g ? ' ft-g2' : ''}`, scene, l.text.trim());
    }
    const line = el('div', 'ft-line', scene);
    line.style.setProperty('--ft-len', String(len));
    for (const w of words) put(line, w);
  }
  st.words = spans.map((span, j) => ({ el: span, ms: words[j].ms, on: false }));
  showSubtitle(v.translation?.[i] || '');
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
  if (st.shown[key] === value) return;
  st.shown[key] = value;
  node.style[prop] = value;
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
  if (TEXT_KINDS.has(sec.kind)) {
    let i = -1;
    for (const j of sec.lines) if (lines[j].ms <= ms) i = j;
    if (i < 0) i = sec.lines[0] ?? -1;
    if (i !== st.line) renderLine(sec, i);
    for (const w of st.words) {
      const on = ms >= w.ms;
      if (on !== w.on) {
        w.on = on;
        w.el.classList.toggle('ft-on', on);
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
  // The camera: a slow push over the scene, a drift in the editorial and a breath on the beat.
  const p = clamp((ms - sec.from) / Math.max(1000, sec.to - sec.from), 0, 1);
  const beatPh = grid ? ((((t - grid.off) / grid.beat) % 1) + 1) % 1 : 0.5;
  const push = sec.kind === 'chorus' ? 0.05 : sec.kind === 'editorial' ? 0.03 : 0.015;
  const breath = grid && !light ? Math.exp(-beatPh * 7) * 0.006 * e : 0;
  const drift = sec.kind === 'editorial' ? (sec.right ? 1 : -1) * p * 1.5 : 0;
  setStyle(ui.camera, 'transform', 'transform', `translateX(${drift.toFixed(3)}vw) scale(${(1 + p * push + breath).toFixed(4)})`);
  // The light: the vignette opens with the energy; a flash on each bar line in the chorus.
  setStyle(ui.vignette, 'vignette', 'opacity', (1 - e * 0.45).toFixed(3));
  const barPh = grid ? ((((t - grid.off) / (grid.beat * 4)) % 1) + 1) % 1 : 1;
  setStyle(ui.flash, 'flash', 'opacity', sec.kind === 'chorus' && grid && !light ? (Math.exp(-barPh * 9) * 0.9 * e).toFixed(3) : '0');
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
    ui.playIcon.textContent = v.paused ? '' : '';
    ui.playLabel.textContent = v.paused ? data.texts.play : data.texts.pause;
  }
  if (st.relief && RELIEF_KINDS.has(sec.kind) && Math.abs(shown - st.relief.p) > 0.0004) {
    st.relief.p = shown;
    st.relief.clip.style.transform = `translateX(${((shown - 1) * 100).toFixed(2)}%)`;
    st.relief.inner.style.transform = `translateX(${((1 - shown) * 100).toFixed(2)}%)`;
  }
};

const running = (v) => st.open && !st.ad && !document.hidden && !!v && !v.paused;

const frame = () => {
  st.raf = 0;
  if (!st.open || st.ad || !st.v) return;
  const v = video();
  const go = running(v);
  if (v) draw(v, go);
  ui.root.classList.toggle('ft-still', !go);
  if (go) st.raf = requestAnimationFrame(frame);
  else releaseLevel();
};

// Draws now and goes on while it plays; paused, one frame.
const kick = () => {
  if (st.open && !st.ad && st.v && !st.raf) st.raf = requestAnimationFrame(frame);
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

const open = () => {
  st.open = true;
  st.ad = !!feathertune.ad?.showing();
  if (!st.ad) fly();
  ui.root.classList.toggle('ft-ad', st.ad);
  ui.root.hidden = false;
  cover();
  requestAnimationFrame(() => st.open && ui.root.classList.add('ft-on'));
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
  ui.root.classList.remove('ft-on');
  setTimeout(() => {
    if (st.open) return;
    ui.root.hidden = true;
    ui.camera.replaceChildren();
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
    const key = `${v.id}\u0000${JSON.stringify(v.sections)}`;
    const same = st.open && key === st.key;
    st.v = v;
    st.key = key;
    st.shown = { time: '', paused: null, transform: '', vignette: '', flash: '', played: -1 };
    const playing = video();
    st.total = playing ? feathertune.clock.duration(playing) || 0 : 0;
    renderTimeline();
    renderRelief();
    setField();
    // The same script keeps its scene on screen (the line is drawn again: a translation may have come).
    if (same) st.line = -2;
    else st.sec = -1;
    if (!st.open) open();
    ui.hint.textContent = v.texts.esc;
    ui.leaveLabel.textContent = v.texts.leave;
    kick();
  },
});

// The display font (Bricolage Grotesque, SIL Open Font License), loaded with the FontFace API so the page
// fetches nothing; one weight range covers every weight the stage uses.
feathertune.stageFont = Object.freeze({
  set(data) {
    if (typeof data?.data !== 'string') return;
    try {
      const bytes = Uint8Array.from(atob(data.data), (c) => c.charCodeAt(0));
      const face = new FontFace('feathertune-stage', bytes.buffer, { weight: '200 800' });
      face
        .load()
        .then(() => document.fonts.add(face))
        .catch((e) => console.warn('feathertune: the stage font did not load', e));
    } catch (e) {
      console.warn('feathertune: the stage font did not load', e);
    }
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
