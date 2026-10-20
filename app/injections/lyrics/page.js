// Synced lyrics. Rust sends { id, lines: [[ms, text, words?], ...] } for the current track, or null (the
// track changed, or LRCLIB has none). In the site's Lyrics tab the timed lines replace the site's own text
// and the current line follows the <video> time; when a line has words ([[ms, text], ...]), they light up
// one by one in the current line. Lines fade with their distance from the current one (`data-d`), and a
// click on a line seeks the <video> to it. With no data the site's own lyrics are left alone. The page
// fetches nothing and calls nothing; lines are shown with textContent only.
const TAB = 'ytmusic-tab-renderer#tab-renderer';
const LYRICS_PAGE = 'MUSIC_PAGE_TYPE_TRACK_LYRICS';
const MAX_LINES = 2000;
const MAX_WORDS = 100;
const MANUAL_SCROLL_MS = 4000;

let lines = null; // [{ ms, text }], sorted by ms
let box = null; // our container, first child of the tab renderer
let items = [];
let current = -1;
let sung = -1; // in the current line, the last word whose time has come
let manualUntil = 0;
let observed = null;
const observer = new MutationObserver(() => refresh());

const tab = () => document.querySelector(TAB);
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

const timed = (l) => Array.isArray(l) && Number.isFinite(l[0]) && l[0] >= 0 && typeof l[1] === 'string';

const valid = (data) =>
  !!data &&
  Array.isArray(data.lines) &&
  data.lines.length > 0 &&
  data.lines.length <= MAX_LINES &&
  data.lines.every(
    (l) =>
      timed(l) && (l[2] === undefined || (Array.isArray(l[2]) && l[2].length <= MAX_WORDS && l[2].every(timed))),
  );

const clear = () => {
  lines = null;
  current = -1;
  sung = -1;
  items = [];
  box?.remove();
  box = null;
  tab()?.removeAttribute('data-feathertune-lyrics');
};

const build = () => {
  box = document.createElement('div');
  box.id = 'feathertune-lyrics';
  const onManual = () => (manualUntil = Date.now() + MANUAL_SCROLL_MS);
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
  items = lines.map((l) => {
    const el = document.createElement('p');
    el.className = l.text ? 'ft-line' : 'ft-line ft-gap';
    if (l.words) {
      el.classList.add('ft-words');
      for (const w of l.words) {
        const span = document.createElement('span');
        span.textContent = w.text;
        el.append(span);
      }
    } else {
      el.textContent = l.text;
    }
    box.append(el);
    return el;
  });
  const credit = document.createElement('p');
  credit.className = 'ft-credit';
  credit.textContent = 'Synced lyrics: LRCLIB';
  box.append(credit);
  current = -1;
  sung = -1;
};

// Puts the box in the tab renderer and shows it only while the Lyrics tab is the open one.
const refresh = () => {
  if (!lines) return;
  const t = tab();
  if (!t) return;
  if (observed !== t) {
    observer.disconnect();
    observer.observe(t, { attributes: true, attributeFilter: ['page-type'] });
    observed = t;
  }
  if (!box) build();
  if (box.parentNode !== t) t.prepend(box);
  const open = t.getAttribute('page-type') === LYRICS_PAGE;
  box.hidden = !open;
  t.toggleAttribute('data-feathertune-lyrics', open);
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

// Lights the words of the current line up to the time (only the spans that change).
const singWords = (ms) => {
  const words = lines[current]?.words;
  if (!words) return;
  const index = indexAt(words, ms);
  if (index === sung) return;
  const spans = items[current].children;
  for (let i = Math.min(index, sung) + 1; i <= Math.max(index, sung); i++) {
    spans[i]?.classList.toggle('ft-sung', i <= index);
  }
  sung = index;
};

// Distance from the current line, for the fade (0 to REACH; farther lines keep the default look).
// Only the lines around the old and the new current one are touched.
const REACH = 4;
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

const sync = (video, force) => {
  if (!lines || !box || box.hidden || !video) return;
  const ms = video.currentTime * 1000 + 250;
  const index = indexAt(lines, ms);
  if (index === current && !force) {
    singWords(ms);
    return;
  }
  const old = items[current];
  old?.classList.remove('ft-current');
  if (old) for (const span of old.children) span.classList.remove('ft-sung');
  const before = current;
  current = index;
  sung = -1;
  singWords(ms);
  fade(before, index);
  const el = items[index];
  if (!el) return;
  el.classList.add('ft-current');
  const t = tab();
  if (!t || Date.now() < manualUntil) return;
  const top = el.getBoundingClientRect().top - t.getBoundingClientRect().top + t.scrollTop;
  t.scrollTo({ top: top - t.clientHeight / 2 + el.clientHeight / 2, behavior: reduced() ? 'auto' : 'smooth' });
};

// Media events do not bubble, but a capturing listener on the document gets them (same as the bridge).
const onTime = (e) => {
  if (!lines || !(e.target instanceof HTMLVideoElement)) return;
  if (!box || !box.isConnected || box.parentNode !== tab()) refresh();
  else sync(e.target, e.type === 'seeked');
};
for (const type of ['timeupdate', 'seeked']) document.addEventListener(type, onTime, true);

feathertune.lyrics = Object.freeze({
  set(data) {
    clear();
    if (!valid(data)) return;
    lines = data.lines.map(([ms, text, words]) => ({ ms, text, words: words?.map(([ms, text]) => ({ ms, text })) }));
    refresh();
  },
});
