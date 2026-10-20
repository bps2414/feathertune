// Synced lyrics. Rust sends { id, lines: [[ms, text], ...] } for the current track, or null (the track
// changed, or LRCLIB has none). In the site's Lyrics tab the timed lines replace the site's own text and
// the current line follows the <video> time. With no data the site's own lyrics are left alone. The page
// fetches nothing and calls nothing; lines are shown with textContent only. Presentation only.
const TAB = 'ytmusic-tab-renderer#tab-renderer';
const LYRICS_PAGE = 'MUSIC_PAGE_TYPE_TRACK_LYRICS';
const MAX_LINES = 2000;
const MANUAL_SCROLL_MS = 4000;

let lines = null; // [{ ms, text }], sorted by ms
let box = null; // our container, first child of the tab renderer
let items = [];
let current = -1;
let manualUntil = 0;
let observed = null;
const observer = new MutationObserver(() => refresh());

const tab = () => document.querySelector(TAB);
const reduced = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

const valid = (data) =>
  !!data &&
  Array.isArray(data.lines) &&
  data.lines.length > 0 &&
  data.lines.length <= MAX_LINES &&
  data.lines.every((l) => Array.isArray(l) && Number.isFinite(l[0]) && l[0] >= 0 && typeof l[1] === 'string');

const clear = () => {
  lines = null;
  current = -1;
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
  items = lines.map((l) => {
    const el = document.createElement('p');
    el.className = l.text ? 'ft-line' : 'ft-line ft-gap';
    el.textContent = l.text;
    box.append(el);
    return el;
  });
  const credit = document.createElement('p');
  credit.className = 'ft-credit';
  credit.textContent = 'Synced lyrics: LRCLIB';
  box.append(credit);
  current = -1;
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

// The last line whose time has come.
const indexAt = (ms) => {
  let lo = 0;
  let hi = lines.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].ms <= ms) {
      found = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }
  return found;
};

const sync = (video, force) => {
  if (!lines || !box || box.hidden || !video) return;
  const index = indexAt(video.currentTime * 1000 + 250);
  if (index === current && !force) return;
  items[current]?.classList.remove('ft-current');
  current = index;
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
    lines = data.lines.map(([ms, text]) => ({ ms, text }));
    refresh();
  },
});
