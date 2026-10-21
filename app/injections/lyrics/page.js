// Synced lyrics. Rust sends { id, lines: [[ms, text, words?], ...], source, estimated } for the current
// track, or null (the track changed, or no source has any). `estimated`: the word times are a guess. In the site's Lyrics tab the timed lines replace the site's own text
// and the current line follows the <video> time; when a line has words ([[ms, text], ...]), they light up
// one by one in the current line. Lines fade with their distance from the current one (`data-d`), and a
// click on a line seeks the <video> to it. While the song plays and the window is in front, the list scrolls
// on its own, in time: the view parks on the current line and moves on to the next one around the
// switch (eased, a little before and a little after), and eases to a new place after a seek, a click or
// a manual scroll (one animation
// frame loop, only while that is so; it writes the scroll only when it moves by a whole device pixel).
// Otherwise each new line scrolls smoothly to the middle. The box goes into the tab renderer showing
// the Lyrics page (there can be more than one, e.g. with an album page open), checked again when a
// `page-type` changes anywhere and on the video's time events. With no data the site's own lyrics are
// left alone. The page fetches nothing and calls nothing; lines are shown with textContent only.
const TAB = 'ytmusic-tab-renderer#tab-renderer';
const LYRICS_PAGE = 'MUSIC_PAGE_TYPE_TRACK_LYRICS';
const MAX_LINES = 2000;
const MAX_WORDS = 100;
const MANUAL_SCROLL_MS = 4000;
// How fast the scroll catches up with where it should be (seconds to cover about two thirds of the way).
const FOLLOW_S = 0.25;
// The move from one line to the next takes this share of the line that ends (at most the cap) and this
// share of the line that starts, so when a line ends the view is still a little short of the middle
// between the two.
const MOVE_BEFORE = 0.4;
const MOVE_BEFORE_MAX_MS = 1600;
const MOVE_AFTER = 0.45;
const MOVE_AFTER_MAX_MS = 1800;

let lines = null; // [{ ms, text }], sorted by ms
let meta = { source: '', estimated: false };
let box = null; // our container, first child of the tab renderer
let items = [];
let current = -1;
let sung = -1; // in the current line, the last word whose time has come
let manualUntil = 0;
const observer = new MutationObserver(() => refresh());
let centers = null; // middle of each line in the tab's scroll space, measured when needed
const resizes = new ResizeObserver(() => (centers = null));
let frame = 0;
let last = 0; // time of the previous frame
let pos = 0; // the scroll position the loop is at, in CSS px
let seen = 0; // the tab's scroll after the loop's last frame; anything else moved it by hand

const tab = () => document.querySelector(`${TAB}[page-type="${LYRICS_PAGE}"]`) ?? document.querySelector(TAB);
const isOpen = (t) => t.getAttribute('page-type') === LYRICS_PAGE;

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
  centers = null;
  resizes.disconnect();
  observer.disconnect();
  box?.remove();
  box = null;
  for (const t of document.querySelectorAll('[data-feathertune-lyrics]')) t.removeAttribute('data-feathertune-lyrics');
};

const build = () => {
  box = document.createElement('div');
  box.id = 'feathertune-lyrics';
  box.classList.toggle('ft-estimated', meta.estimated);
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
  credit.textContent = `Synced lyrics: ${meta.source}${meta.estimated ? ' · word timing estimated' : ''}`;
  box.append(credit);
  current = -1;
  sung = -1;
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
    box.parentElement?.removeAttribute('data-feathertune-lyrics');
    t.prepend(box);
  }
  const open = isOpen(t);
  box.hidden = !open;
  t.toggleAttribute('data-feathertune-lyrics', open);
  if (open) {
    run();
    sync(document.querySelector('video'), true);
  }
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
  if (frame || !t || Date.now() < manualUntil) return;
  t.scrollTo({ top: center(index) - t.clientHeight / 2, behavior: 'smooth' });
};

const center = (i) => {
  if (!centers) {
    const t = tab();
    const origin = t.getBoundingClientRect().top - t.scrollTop;
    centers = items.map((el) => {
      const r = el.getBoundingClientRect();
      return r.top - origin + r.height / 2;
    });
  }
  return centers[i] ?? 0;
};

// The move from line k-1 to line k, in the time around the switch to k: it starts before the switch
// (a share of line k-1, capped) and ends after it (a share of line k, capped). In between the view parks
// on the current line; lines shorter than that keep the view moving all the time.
const move = (k) => {
  const before = lines[k].ms - lines[k - 1].ms;
  const after = k + 1 < lines.length ? lines[k + 1].ms - lines[k].ms : MOVE_AFTER_MAX_MS;
  const from = lines[k].ms - Math.min(MOVE_BEFORE_MAX_MS, before * MOVE_BEFORE);
  return { from, to: lines[k].ms + Math.min(MOVE_AFTER_MAX_MS, after * MOVE_AFTER) };
};

// Where the middle of the view should be now: parked on the current line, or on the way between two.
const goal = (ms) => {
  const i = indexAt(lines, ms);
  if (i < 0) return center(0);
  // The move to the next line may have started; the one to this line may not have ended.
  for (const k of [i + 1, i]) {
    if (k < 1 || k >= lines.length) continue;
    const { from, to } = move(k);
    if (ms < from || ms >= to) continue;
    const p = (ms - from) / (to - from);
    return center(k - 1) + (center(k) - center(k - 1)) * p * p * (3 - 2 * p);
  }
  return center(i);
};

const media = () => document.querySelector('video');
const canRun = (v) =>
  !!lines && !!box && box.isConnected && !box.hidden && !!v && !v.paused && !document.hidden && document.hasFocus();

// `frame` stays set while a frame runs, so `sync` leaves the scroll to the loop.
const step = (now) => {
  const v = media();
  const t = tab();
  if (!t || !canRun(v)) {
    frame = 0;
    return;
  }
  sync(v, false);
  const dt = Math.min(0.1, (now - last) / 1000);
  last = now;
  if (Math.abs(t.scrollTop - seen) > 1) manualUntil = Date.now() + MANUAL_SCROLL_MS;
  if (Date.now() < manualUntil) {
    pos = t.scrollTop;
  } else {
    const target = goal(v.currentTime * 1000 + 250) - t.clientHeight / 2;
    pos += (target - pos) * (1 - Math.exp(-dt / FOLLOW_S));
    const dpr = devicePixelRatio || 1;
    const top = Math.round(pos * dpr) / dpr;
    if (Math.abs(top - t.scrollTop) * dpr >= 1) t.scrollTop = top;
  }
  seen = t.scrollTop;
  frame = requestAnimationFrame(step);
};

// Starts the loop when it has something to follow; it stops on its own when that ends.
const run = () => {
  const t = tab();
  if (frame || !t || !canRun(media())) return;
  pos = seen = t.scrollTop;
  last = performance.now();
  frame = requestAnimationFrame(step);
};

// Media events do not bubble, but a capturing listener on the document gets them (same as the bridge).
const onTime = (e) => {
  if (!lines || !(e.target instanceof HTMLVideoElement)) return;
  const t = tab();
  if (!box || !t || box.parentNode !== t || box.hidden === isOpen(t)) {
    refresh();
    return;
  }
  run();
  sync(e.target, e.type === 'seeked');
};
for (const type of ['timeupdate', 'seeked', 'play']) document.addEventListener(type, onTime, true);
window.addEventListener('focus', run);
document.addEventListener('visibilitychange', run);

feathertune.lyrics = Object.freeze({
  set(data) {
    clear();
    if (!valid(data)) return;
    meta = {
      source: typeof data.source === 'string' ? data.source.slice(0, 40) : '',
      estimated: data.estimated === true,
    };
    lines = data.lines.map(([ms, text, words]) => ({ ms, text, words: words?.map(([ms, text]) => ({ ms, text })) }));
    refresh();
  },
});
