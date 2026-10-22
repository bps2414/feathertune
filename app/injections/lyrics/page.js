// Synced lyrics. Rust sends { id, lines: [[ms, text, words?, end?, bg?], ...], estimated, translation?, credit }
// for the current track, or null (the track changed, or no source has any). `credit` is the line under the
// lyrics (sources, in the app's language). `end`: when the line stops being sung (0 or missing: when the
// next one starts); `bg`: background vocals sung with it, as words ([[ms, text], ...]) shown under it in a
// smaller type.
// `estimated`: the word times are a guess. `translation` has one text (or null) per line, shown under
// it in a smaller type; it lights with its line as a whole, never word by word. In the site's Lyrics tab the timed lines replace the site's own text
// and the lines being sung follow the <video> time. Lines overlap in fast songs and with echoes (a line
// starts before the one before it ends), so every line from its start to its end is lit at once (the last
// one to start always is); when a line has words ([[ms, text], ...]), they light up
// one by one in each lit line, each fading in over a share of its own length (fast words at once), and
// a one-shot timer wakes the page when the next word, line or end is due. Lines fade with their distance from
// the latest one (`data-d`), and a click on a line seeks the <video> to it. While the song plays and the
// window is in front, the list scrolls on its own, in time, on one smooth curve: the view rests on a line
// with time to spare and moves on to the next one around the switch (a little before and a little after),
// glides through lines too short to rest on (in a fast part the curve is averaged over a few lines, so the
// view flows at the part's pace instead of jerking with each line), and eases to a new place after a seek,
// a click or a manual scroll. The box glides as one transform animation that the compositor runs (see
// `start`), only while that is so; otherwise each new line scrolls smoothly to the middle. The box goes
// into the tab renderer showing the Lyrics page (there can be more than one, e.g. with an album page open), checked again when a
// `page-type` changes anywhere and on the video's time events. With no data the site's own lyrics are
// left alone. The page fetches nothing and calls nothing; lines are shown with textContent only.
const TAB = 'ytmusic-tab-renderer#tab-renderer';
const LYRICS_PAGE = 'MUSIC_PAGE_TYPE_TRACK_LYRICS';
const MAX_LINES = 2000;
const MAX_WORDS = 100;
const MAX_TRANSLATION_CHARS = 600;
const MANUAL_SCROLL_MS = 4000;
// Lines and words change this much before their time.
const LEAD_MS = 250;
// A line stays lit this long after its last word (on the clock with the lead), so the word is heard
// before the line dims.
const LINGER_MS = 350;
// How many lines back from the latest one can still be sung.
const ACTIVE_SCAN = 8;
// A sung word fades in over this share of its length (at most the CSS duration).
const WORD_FADE = 0.4;
// After a seek, a click, a manual scroll or a resize, how fast the scroll catches up with the curve
// (seconds to cover about two thirds of the way); otherwise it is on the curve.
const FOLLOW_S = 0.25;
// The glide is a list of keyframes the compositor joins with straight lines: they are at most this far
// (CSS px) from the curve. Its clock is set back to the song's when they drift apart by more than DRIFT_MS.
const GLIDE_TOLERANCE_PX = 0.2;
const DRIFT_MS = 50;
// The move from one line to the next takes this share of the line that ends (at most the cap) and this
// share of the line that starts, so when a line ends the view is still a little short of the middle
// between the two. A line left with less rest than PARK_MIN_MS is passed through without stopping.
const MOVE_BEFORE = 0.4;
const MOVE_BEFORE_MAX_MS = 1600;
const MOVE_AFTER = 0.45;
const MOVE_AFTER_MAX_MS = 1800;
const PARK_MIN_MS = 600;
// In a fast part (lines less than DENSE_GAP_MS apart on average) the curve is averaged over a window of
// SMOOTH_LINES of those gaps (at most SMOOTH_MAX_MS), twice, so the view flows at the part's pace instead
// of speeding up and slowing down with each line; the window itself changes over SMOOTH_EASE_MS (at least
// half of SMOOTH_MAX_MS, so its edges only move forward and the view never turns back), and slower parts
// keep the curve as it is. The averaged curve is sampled every SAMPLE_MS.
const DENSE_GAP_MS = 1200;
const SMOOTH_LINES = 4;
const SMOOTH_MAX_MS = 3000;
const SMOOTH_EASE_MS = 2000;
const SAMPLE_MS = 20;

let lines = null; // [{ ms, text, words?, end, bg?, translation? }], sorted by ms
let ends = []; // when each line stops being sung (see `lineEnd`)
let keys = []; // [{ ms, i }]: the scroll curve passes the middle of line i at ms (see `curveAt`)
let radii = null; // per sample from `sampleStart`, half the averaging window in samples; null: no fast part
let sampleStart = 0;
let smooth = null; // the averaged curve at those samples, built when needed from the measured lines
let meta = { credit: '', estimated: false };
let box = null; // our container, first child of the tab renderer
let items = [];
let spans = []; // per line: { main: [span], bg: [span] }, the word elements
let current = -1; // the latest line to start
let active = []; // the lines lit now: the latest and those before it not yet over
let sungMain = []; // per line, the last word whose time has come (-1: none)
let sungBg = [];
let manualUntil = 0;
const observer = new MutationObserver(() => refresh());
let centers = null; // middle of each line below the top of the box, measured when needed
let anim = null; // the box's glide along the curve (see `start`), while it follows the song
let animStart = 0; // the song time (with the lead) at the glide's time 0
let ownTop = 0; // the tab's scroll while the box glides; anything else moved it by hand
let pointing = 0; // timer: the pointer stopped moving over the box (see `build`)
const POINTER_IDLE_MS = 800;
const resizes = new ResizeObserver(() => {
  centers = null;
  if (anim) start(media());
});
let timer = 0;
let due = -1; // the time (with the lead) the timer is set for

const tab = () => document.querySelector(`${TAB}[page-type="${LYRICS_PAGE}"]`) ?? document.querySelector(TAB);
const isOpen = (t) => t.getAttribute('page-type') === LYRICS_PAGE;

const timed = (l) => Array.isArray(l) && Number.isFinite(l[0]) && l[0] >= 0 && typeof l[1] === 'string';
const wordList = (w) => w === undefined || w === null || (Array.isArray(w) && w.length <= MAX_WORDS && w.every(timed));

const valid = (data) =>
  !!data &&
  Array.isArray(data.lines) &&
  data.lines.length > 0 &&
  data.lines.length <= MAX_LINES &&
  data.lines.every(
    (l) =>
      timed(l) &&
      wordList(l[2]) &&
      (l[3] === undefined || (Number.isFinite(l[3]) && l[3] >= 0)) &&
      wordList(l[4]),
  ) &&
  (data.translation === undefined ||
    (Array.isArray(data.translation) &&
      data.translation.length === data.lines.length &&
      data.translation.every((t) => t === null || (typeof t === 'string' && t.length <= MAX_TRANSLATION_CHARS))));

const clear = () => {
  lines = null;
  ends = [];
  keys = [];
  radii = smooth = null;
  current = -1;
  active = [];
  sungMain = [];
  sungBg = [];
  spans = [];
  items = [];
  centers = null;
  clearTimeout(timer);
  timer = 0;
  resizes.disconnect();
  observer.disconnect();
  anim?.cancel();
  anim = null;
  clearTimeout(pointing);
  box?.remove();
  box = null;
  for (const t of document.querySelectorAll('[data-feathertune-lyrics]')) t.removeAttribute('data-feathertune-lyrics');
  document.documentElement.removeAttribute('data-feathertune-lyrics-open');
};

const build = () => {
  box = document.createElement('div');
  box.id = 'feathertune-lyrics';
  box.classList.toggle('ft-estimated', meta.estimated);
  // The pointer's hover is not updated under content the compositor moves, so while the box glides a line
  // shows it only while the pointer moves (style.css); a still pointer would leave it on a line gliding away.
  const point = () => {
    box.classList.add('ft-pointing');
    clearTimeout(pointing);
    pointing = setTimeout(() => box?.classList.remove('ft-pointing'), POINTER_IDLE_MS);
  };
  box.addEventListener('mousemove', point, { passive: true });
  box.addEventListener('mouseleave', () => box.classList.remove('ft-pointing'));
  box.addEventListener('wheel', onManual, { passive: true });
  box.addEventListener('touchmove', onManual, { passive: true });
  box.addEventListener('click', (e) => {
    const index = items.indexOf(e.target instanceof Element ? e.target.closest('.ft-line') : null);
    const video = document.querySelector('video');
    if (index < 0 || !lines[index].text || !video) return;
    manualUntil = 0;
    video.currentTime = lines[index].ms / 1000;
    sync(video, true);
  });
  spans = [];
  items = lines.map((l, i) => {
    const el = document.createElement('p');
    el.className = l.text ? 'ft-line' : 'ft-line ft-gap';
    // The glow is a copy of the text on a layer of its own (style.css).
    el.dataset.text = l.text;
    const own = { main: [], bg: [] };
    // Each word fades in over a share of its own length; the last one ends with the line.
    const wordSpans = (parent, words) =>
      words.map((w, j) => {
        const span = document.createElement('span');
        span.textContent = w.text;
        const end = words[j + 1]?.ms ?? ends[i];
        if (end > w.ms) span.style.setProperty('--ft-word', `${Math.round((end - w.ms) * WORD_FADE)}ms`);
        parent.append(span);
        return span;
      });
    if (l.words) {
      el.classList.add('ft-words');
      own.main = wordSpans(el, l.words);
    } else {
      el.textContent = l.text;
    }
    if (l.bg) {
      const bg = document.createElement('span');
      bg.className = 'ft-bg';
      own.bg = wordSpans(bg, l.bg);
      el.append(bg);
    }
    spans.push(own);
    if (l.translation) {
      const tr = document.createElement('span');
      tr.className = 'ft-tr';
      tr.textContent = l.translation;
      el.append(tr);
    }
    box.append(el);
    return el;
  });
  const credit = document.createElement('p');
  credit.className = 'ft-credit';
  credit.textContent = meta.credit;
  box.append(credit);
  current = -1;
  active = [];
  sungMain = lines.map(() => -1);
  sungBg = lines.map(() => -1);
  centers = null;
  resizes.observe(box);
};

// Puts the box in the tab renderer and shows it only while the Lyrics tab is the open one.
const refresh = () => {
  if (!lines) return;
  observer.observe(document.body, { attributes: true, attributeFilter: ['page-type'], subtree: true });
  const t = tab();
  if (!t) return;
  if (!box) build();
  if (box.parentNode !== t) {
    anim?.cancel();
    anim = null;
    box.parentElement?.removeAttribute('data-feathertune-lyrics');
    t.prepend(box);
  }
  const open = isOpen(t);
  box.hidden = !open;
  t.toggleAttribute('data-feathertune-lyrics', open);
  // Other modules ease off while our lyrics show (ambient/style.css).
  document.documentElement.toggleAttribute('data-feathertune-lyrics-open', open);
  follow();
  if (open) sync(document.querySelector('video'), true);
};

// The last entry (line or word) whose time has come.
const indexAt = (list, ms) => {
  let lo = 0;
  let hi = list.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (list[mid].ms <= ms) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
};

// When line `i` stops being sung: its own end, else when the next one starts (never before its last word).
const lineEnd = (i) => {
  const l = lines[i];
  const lastWord = Math.max(l.words?.at(-1)?.ms ?? 0, l.bg?.at(-1)?.ms ?? 0);
  const end = l.end > l.ms ? l.end : (lines[i + 1]?.ms ?? l.ms + 4000);
  return Math.max(end, lastWord + 100);
};

// Moves the lit words of one line (`spans`) from the word `from` to the word `to`.
const toggleWords = (list, from, to) => {
  for (let i = Math.min(from, to) + 1; i <= Math.max(from, to); i++) list[i]?.classList.toggle('ft-sung', i <= to);
};

// Lights the words of line `i` up to the time (only the spans that change).
const singWords = (i, ms) => {
  const l = lines[i];
  if (l.words) {
    const index = indexAt(l.words, ms);
    if (index !== sungMain[i]) {
      toggleWords(spans[i].main, sungMain[i], index);
      sungMain[i] = index;
    }
  }
  if (l.bg) {
    const index = indexAt(l.bg, ms);
    if (index !== sungBg[i]) {
      toggleWords(spans[i].bg, sungBg[i], index);
      sungBg[i] = index;
    }
  }
};

// The lines lit at `ms` (with `index` the latest to start): the latest one, kept lit until the next
// starts, and the ones before it still being sung.
const activeAt = (ms, index) => {
  const out = [];
  for (let i = Math.max(0, index - ACTIVE_SCAN); i <= index; i++) {
    if (i === index || ms < ends[i] + LINGER_MS) out.push(i);
  }
  return out;
};

const dim = (i) => {
  items[i].classList.remove('ft-current');
  toggleWords(spans[i].main, sungMain[i], -1);
  toggleWords(spans[i].bg, sungBg[i], -1);
  sungMain[i] = sungBg[i] = -1;
};

const light = (next) => {
  if (next.length === active.length && next.every((i, k) => i === active[k])) return;
  for (const i of active) if (!next.includes(i)) dim(i);
  for (const i of next) if (!active.includes(i)) items[i].classList.add('ft-current');
  active = next;
};

// Distance from the current line (0 to REACH; farther lines keep the default look). style.css fades the
// lines up to 4 away and makes every line with a distance a layer of its own, so a line becomes one (and
// draws its glow) about three lines before it is near enough to fade, mostly off screen, instead of at the
// switch that brings it on. Only the lines around the old and the new current one are touched.
const REACH = 7;
const fade = (from, to) => {
  const near = new Set();
  for (const center of [from, to]) {
    if (center < 0) continue;
    for (let i = Math.max(0, center - REACH - 1); i <= Math.min(items.length - 1, center + REACH + 1); i++) near.add(i);
  }
  for (const i of near) {
    const d = to < 0 ? REACH + 1 : Math.abs(i - to);
    if (d > REACH) delete items[i].dataset.d;
    else items[i].dataset.d = String(d);
  }
};

// Wakes `sync` when the next word, line start or line end is due (`ms`: now, with the lead), so words
// light on time without a frame loop, also while the window is shown but not in front. One timer, set
// only while playing and shown; it is set again only when what is due next changes (or `force`).
const arm = (video, ms, force) => {
  let next = lines[current + 1]?.ms ?? Infinity;
  for (const i of active) {
    next = Math.min(next, lines[i].words?.[sungMain[i] + 1]?.ms ?? Infinity, lines[i].bg?.[sungBg[i] + 1]?.ms ?? Infinity);
    if (i !== current) next = Math.min(next, ends[i] + LINGER_MS);
  }
  if (!Number.isFinite(next) || video.paused || document.hidden || !(video.playbackRate > 0)) {
    clearTimeout(timer);
    timer = 0;
    return;
  }
  if (timer && next === due && !force) return;
  clearTimeout(timer);
  due = next;
  timer = setTimeout(
    () => {
      timer = 0;
      sync(video, false);
    },
    Math.max(0, next - ms) / video.playbackRate + 1,
  );
};

const sync = (video, force) => {
  if (!lines || !box || box.hidden || !video) return;
  const ms = video.currentTime * 1000 + LEAD_MS;
  const index = indexAt(lines, ms);
  light(activeAt(ms, index));
  for (const i of active) singWords(i, ms);
  if (index === current && !force) {
    arm(video, ms, false);
    return;
  }
  const before = current;
  current = index;
  fade(before, index);
  arm(video, ms, true);
  const el = items[index];
  if (!el) return;
  const t = tab();
  if (anim || !t || Date.now() < manualUntil) return;
  const top = box.getBoundingClientRect().top - t.getBoundingClientRect().top + t.scrollTop;
  t.scrollTo({ top: top + center(index) - t.clientHeight / 2, behavior: 'smooth' });
};

// Lines scale about their vertical middle and the box moves as a whole, so these hold while either runs.
const center = (i) => {
  if (!centers) {
    const top = box.getBoundingClientRect().top;
    centers = items.map((el) => {
      const r = el.getBoundingClientRect();
      return r.top - top + r.height / 2;
    });
    smooth = null;
  }
  return centers[i] ?? 0;
};

// The keys of the scroll curve. The view arrives on line k a share of it after it starts (capped; line 0
// from its start) and leaves it a share of it before the next one (capped), so the move from one line
// to the next spans the switch. A line with at least PARK_MIN_MS between the two gets both keys and the
// view rests on it; a shorter one gets one key in the middle and the view glides through it.
const buildKeys = () => {
  keys = [];
  const add = (ms, i) => {
    while (keys.length && keys[keys.length - 1].ms >= ms) keys.pop();
    keys.push({ ms, i });
  };
  lines.forEach((l, k) => {
    const next = lines[k + 1];
    const len = next ? next.ms - l.ms : MOVE_AFTER_MAX_MS;
    const arrive = k === 0 ? l.ms : l.ms + Math.min(MOVE_AFTER_MAX_MS, len * MOVE_AFTER);
    if (!next) return add(arrive, k);
    const leave = next.ms - Math.min(MOVE_BEFORE_MAX_MS, len * MOVE_BEFORE);
    if (leave - arrive >= PARK_MIN_MS) {
      add(arrive, k);
      add(leave, k);
    } else {
      add((arrive + leave) / 2, k);
    }
  });
  buildRadii();
};

// At each sample `s`, the mean of `src` from `r[s]` samples before it to `r[s]` after (a narrower window
// near the ends, so it stays centered).
const average = (src, r) => {
  const n = src.length;
  const sums = new Float64Array(n + 1);
  for (let s = 0; s < n; s++) sums[s + 1] = sums[s] + src[s];
  const out = new Float64Array(n);
  for (let s = 0; s < n; s++) {
    const h = Math.min(r[s], s, n - 1 - s);
    out[s] = (sums[s + h + 1] - sums[s - h]) / (2 * h + 1);
  }
  return out;
};

// The averaging window at each sample, from the mean gap of the lines around it (see DENSE_GAP_MS).
const buildRadii = () => {
  radii = smooth = null;
  const n = lines.length;
  if (!keys.length) return;
  sampleStart = keys[0].ms - SMOOTH_MAX_MS;
  const count = Math.ceil((keys[keys.length - 1].ms + SMOOTH_MAX_MS - sampleStart) / SAMPLE_MS) + 1;
  const widths = new Float64Array(count);
  let dense = false;
  for (let s = 0; s < count; s++) {
    const i = Math.max(0, indexAt(lines, sampleStart + s * SAMPLE_MS));
    const a = Math.max(0, i - 2);
    const b = Math.min(n - 1, i + 2);
    const gap = b > a ? (lines[b].ms - lines[a].ms) / (b - a) : Infinity;
    if (gap < DENSE_GAP_MS) {
      widths[s] = Math.min(SMOOTH_MAX_MS, gap * SMOOTH_LINES);
      dense = true;
    }
  }
  if (!dense) return;
  const ease = new Uint32Array(count).fill(Math.round(SMOOTH_EASE_MS / 2 / SAMPLE_MS));
  radii = Uint32Array.from(average(widths, ease), (w) => Math.round(w / 2 / SAMPLE_MS));
};

const keyY = (j) => center(keys[j].i);
const secant = (j) => (keyY(j + 1) - keyY(j)) / (keys[j + 1].ms - keys[j].ms);
// The curve's slope at key j: flat at the ends and where it rests, else a weighted harmonic mean of the
// two sides (monotone cubic interpolation, so the view never overshoots a line or turns back).
const slope = (j) => {
  if (j === 0 || j === keys.length - 1) return 0;
  const d0 = secant(j - 1);
  const d1 = secant(j);
  if (d0 * d1 <= 0) return 0;
  const h0 = keys[j].ms - keys[j - 1].ms;
  const h1 = keys[j + 1].ms - keys[j].ms;
  return (3 * (h0 + h1)) / ((2 * h1 + h0) / d0 + (h1 + 2 * h0) / d1);
};

// The curve through the keys at `ms`, smooth in place and speed, so the view never stops between lines
// it has no time to rest on.
const curveAt = (ms) => {
  const n = keys.length;
  if (!n) return center(0);
  if (ms <= keys[0].ms) return keyY(0);
  if (ms >= keys[n - 1].ms) return keyY(n - 1);
  const j = indexAt(keys, ms);
  const h = keys[j + 1].ms - keys[j].ms;
  const s = (ms - keys[j].ms) / h;
  const s2 = s * s;
  const s3 = s2 * s;
  return (
    (2 * s3 - 3 * s2 + 1) * keyY(j) +
    (s3 - 2 * s2 + s) * h * slope(j) +
    (3 * s2 - 2 * s3) * keyY(j + 1) +
    (s3 - s2) * h * slope(j + 1)
  );
};

// Where the middle of the view should be at `ms`: the curve, averaged in fast parts (see `buildRadii`).
const goal = (ms) => {
  if (!radii) return curveAt(ms);
  center(0); // measures the lines if needed, which drops an average built on the old ones
  if (!smooth) {
    const raw = Float64Array.from(radii, (_, s) => curveAt(sampleStart + s * SAMPLE_MS));
    smooth = average(average(raw, radii), radii);
  }
  const f = (ms - sampleStart) / SAMPLE_MS;
  if (f <= 0) return smooth[0];
  if (f >= smooth.length - 1) return smooth[smooth.length - 1];
  const s = Math.floor(f);
  return smooth[s] + (smooth[s + 1] - smooth[s]) * (f - s);
};

const media = () => document.querySelector('video');
const canRun = (v) =>
  !!lines && !!box && box.isConnected && !box.hidden && !!v && !v.paused && !document.hidden && document.hasFocus();

// The indices of the samples `ys` (SAMPLE_MS apart) that straight lines must join to stay within
// GLIDE_TOLERANCE_PX of every sample: each line goes on while one slope still fits all the samples it
// passes (a cone narrowed at each one).
const corners = (ys) => {
  const out = [0];
  let a = 0;
  let lo = -Infinity;
  let hi = Infinity;
  for (let k = 1; k < ys.length; k++) {
    let dt = k - a;
    if ((ys[k] - ys[a]) / dt < lo || (ys[k] - ys[a]) / dt > hi) {
      a = k - 1;
      out.push(a);
      dt = 1;
      lo = -Infinity;
      hi = Infinity;
    }
    lo = Math.max(lo, (ys[k] - GLIDE_TOLERANCE_PX - ys[a]) / dt);
    hi = Math.min(hi, (ys[k] + GLIDE_TOLERANCE_PX - ys[a]) / dt);
  }
  if (ys.length > 1) out.push(ys.length - 1);
  return out;
};

// How far the box is moved now (its glide, or none).
const shown = () => {
  const m = getComputedStyle(box).transform;
  return m && m !== 'none' ? new DOMMatrixReadOnly(m).m42 : 0;
};

// The box glides along the curve as a transform animation from the song's time now to its end, so the
// compositor moves it on its own clock: a busy page no longer drops its frames, and the tab's scroll (and
// the site's scroll listeners) stay still meanwhile. It starts where the box is now and eases onto the
// curve (FOLLOW_S), and its clock follows the song's (rate, drift; see `align`). Started again after a
// seek, a resize or a manual scroll.
const start = (v) => {
  const t = tab();
  if (!t || !canRun(v)) return;
  const from = anim ? shown() : 0;
  anim?.cancel();
  anim = null;
  const viewH = t.clientHeight;
  if (!viewH) return;
  t.scrollTop = t.scrollTop; // ends a smooth scroll of `sync` still under way
  const rel = box.getBoundingClientRect().top - t.getBoundingClientRect().top;
  const ms0 = v.currentTime * 1000 + LEAD_MS;
  const onCurve = (ms) => viewH / 2 - rel - goal(ms);
  const gap = from - onCurve(ms0);
  const curveEnd = radii ? sampleStart + (smooth?.length ?? radii.length) * SAMPLE_MS : keys[keys.length - 1].ms;
  const easeEnd = Math.abs(gap) > GLIDE_TOLERANCE_PX ? ms0 + FOLLOW_S * 1000 * Math.log(Math.abs(gap) / 0.05) : ms0;
  const count = Math.max(2, Math.ceil((Math.max(curveEnd, easeEnd) - ms0) / SAMPLE_MS) + 1);
  const ys = Float64Array.from({ length: count }, (_, s) => {
    const ms = ms0 + s * SAMPLE_MS;
    return onCurve(ms) + gap * Math.exp(-(ms - ms0) / (FOLLOW_S * 1000));
  });
  const frames = corners(ys).map((s) => ({ offset: s / (count - 1), transform: `translateY(${ys[s].toFixed(2)}px)` }));
  anim = box.animate(frames, { duration: (count - 1) * SAMPLE_MS, fill: 'forwards' });
  box.classList.add('ft-gliding');
  anim.playbackRate = v.playbackRate;
  animStart = ms0;
  ownTop = t.scrollTop;
  anim.ready.then(() => align(v)).catch(() => {});
};

// Sets the glide's clock back to the song's when they drift apart (the rate follows the song's too).
const align = (v) => {
  if (!anim) return;
  if (anim.playbackRate !== v.playbackRate) anim.playbackRate = v.playbackRate;
  const want = v.currentTime * 1000 + LEAD_MS - animStart;
  if (Math.abs((anim.currentTime ?? 0) - want) > DRIFT_MS) anim.currentTime = want;
};

// Ends the glide, leaving the view where it is: what the box was moved goes into the tab's scroll.
const stop = () => {
  if (!anim) return;
  const moved = shown();
  anim.cancel();
  anim = null;
  box.classList.remove('ft-gliding');
  const t = box?.parentNode;
  if (t instanceof Element && !box.hidden) {
    t.scrollTop -= moved;
    ownTop = t.scrollTop;
  }
};

// Glides while there is something to follow (the song plays, the tab is shown and in front, no manual
// scroll); otherwise the view stays and `sync` scrolls each new line to the middle.
const follow = () => {
  const v = media();
  if (!canRun(v) || Date.now() < manualUntil) return stop();
  if (anim) align(v);
  else start(v);
};

const onManual = () => {
  manualUntil = Date.now() + MANUAL_SCROLL_MS;
  stop();
};

// Media events do not bubble, but a capturing listener on the document gets them (same as the bridge).
const onTime = (e) => {
  if (!lines || !(e.target instanceof HTMLVideoElement)) return;
  const t = tab();
  if (!box || !t || box.parentNode !== t || box.hidden === isOpen(t)) {
    refresh();
    return;
  }
  if (e.type === 'seeked' && anim) start(e.target);
  follow();
  if (e.type === 'ratechange') {
    clearTimeout(timer);
    timer = 0;
  }
  sync(e.target, e.type === 'seeked');
};
for (const type of ['timeupdate', 'seeked', 'play', 'pause', 'ratechange']) {
  document.addEventListener(type, onTime, true);
}
window.addEventListener('focus', follow);
window.addEventListener('blur', follow);
document.addEventListener('visibilitychange', follow);
// A scroll of the tab while the box glides is a manual one (the scrollbar, keys); scroll events do not
// bubble, so this listens while capturing.
document.addEventListener(
  'scroll',
  (e) => {
    if (anim && e.target === box?.parentNode && Math.abs(e.target.scrollTop - ownTop) > 1) onManual();
  },
  true,
);

feathertune.lyrics = Object.freeze({
  set(data) {
    clear();
    if (!valid(data)) return;
    meta = {
      credit: typeof data.credit === 'string' ? data.credit.slice(0, 160) : '',
      estimated: data.estimated === true,
    };
    const toWords = (words) => (words?.length ? words.map(([ms, text]) => ({ ms, text })) : undefined);
    lines = data.lines.map(([ms, text, words, end, bg], i) => ({
      ms,
      text,
      words: toWords(words),
      end: end ?? 0,
      bg: toWords(bg),
      translation: data.translation?.[i] ?? null,
    }));
    ends = lines.map((_, i) => lineEnd(i));
    buildKeys();
    refresh();
  },
});
