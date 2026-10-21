// Player state bridge (always on). Reads navigator.mediaSession.metadata and the <video> and
// sends them to Rust through the `player_state` command: right away on a track, play/pause or
// seek change, plus a position refresh every 15 s while playing (Rust extrapolates in between).
// Each IPC call is a fetch with a CORS preflight: sending every second cost ~3% CPU (measured).
// With them go where the track plays from (for the mixes): the playlist (`list=` of the URL), the
// next track and, when it changed, the queue (at most 200 tracks), read from the site's state on a
// track change and when the queue's DOM changes (an observer, no polling).
// One-way: Rust never sends anything back through here.
if (window.top !== window) return;

const SELECTORS = {
  // Its classes say when an ad plays.
  player: '#movie_player',
  // Its getState() is the site's state: `queue.items`, `selectedItemIndex`, `automixItems`, `repeatMode`.
  playerBar: 'ytmusic-player-bar',
  queue: '#queue',
};
const REFRESH_MS = 15000;
const MAX_QUEUE = 200;
const QUEUE_SETTLE_MS = 500;
let lastKey = '';
let lastSent = 0;
let lastQueue = '';
let lastWhere = '';
let video = null;

const adShowing = () => {
  const p = document.querySelector(SELECTORS.player);
  return !!p && (p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting'));
};

// While an ad plays the <video> is the ad's, so its duration is not the track's: it goes as 0 (unknown).
const read = (video) => {
  const m = navigator.mediaSession?.metadata;
  if (!m || !m.title) return null;
  const art = m.artwork?.length ? m.artwork[m.artwork.length - 1].src : null;
  const id = new URLSearchParams(location.search).get('v');
  const num = (x) => (Number.isFinite(x) ? Math.round(x * 10) / 10 : 0);
  return {
    id,
    title: m.title,
    artist: m.artist || '',
    album: m.album || '',
    artwork: art,
    duration: adShowing() ? 0 : num(video.duration),
    position: num(video.currentTime),
    paused: video.paused,
  };
};

// A queue entry of the site's state as { id, title, artist }, or null.
const item = (entry) => {
  const r =
    entry?.playlistPanelVideoRenderer || entry?.playlistPanelVideoWrapperRenderer?.primaryRenderer?.playlistPanelVideoRenderer;
  const text = (t) => (t?.runs || []).map((x) => x.text).join('').slice(0, 200);
  if (!r?.videoId || !/^[A-Za-z0-9_-]{11}$/.test(r.videoId)) return null;
  return { id: r.videoId, title: text(r.title), artist: text(r.shortBylineText || r.longBylineText).split(' • ')[0] };
};
// The playlist, the next track (repeat-one: this one; at the end: the first on repeat-all, else
// autoplay's first) and the queue.
const where = () => {
  const list = new URLSearchParams(location.search).get('list');
  const out = { list: list && /^[A-Za-z0-9_-]{1,64}$/.test(list) ? list : null, next: null, queue: [] };
  let q = null;
  try {
    q = document.querySelector(SELECTORS.playerBar)?.getState?.()?.queue;
  } catch {}
  if (!q || !Array.isArray(q.items)) return out;
  const items = q.items.map(item);
  out.queue = items.filter(Boolean).slice(0, MAX_QUEUE);
  const i = q.selectedItemIndex;
  if (Number.isInteger(i)) {
    if (q.repeatMode === 'ONE') out.next = items[i];
    else if (i + 1 < items.length) out.next = items[i + 1];
    else if (q.repeatMode === 'ALL') out.next = items[0];
    else out.next = item(q.automixItems?.[0]);
  }
  out.next ||= null;
  return out;
};

// `queueChanged`: the queue's DOM changed, so the next track and the queue are read again.
const send = (video, force, queueChanged = false) => {
  const s = read(video);
  if (!s) return;
  const key = [s.id, s.title, s.artist, s.album, s.artwork, s.duration, s.paused].join('\u0000');
  const now = performance.now();
  const due = force || (!s.paused && now - lastSent >= REFRESH_MS);
  if (key === lastKey && !due && !queueChanged) return;
  // The site's state is only read here: on a change, not on every timeupdate.
  const w = where();
  const whereKey = [w.list, w.next?.id].join('\u0000');
  if (key === lastKey && !due && whereKey === lastWhere && w.queue.map((x) => x.id).join(',') === lastQueue) return;
  lastKey = key;
  lastWhere = whereKey;
  lastSent = now;
  s.list = w.list;
  s.next = w.next;
  const queueKey = w.queue.map((x) => x.id).join(',');
  if (queueKey !== lastQueue) {
    lastQueue = queueKey;
    if (w.list) s.queue = w.queue;
  }
  window.__TAURI_INTERNALS__?.invoke('player_state', { state: JSON.stringify(s) }).catch(() => {});
};

// The queue's DOM changed (a track added, shuffle, the queue refilled): send what changed, once it settles.
let settle = 0;
let watched = null;
const queueObserver = new MutationObserver(() => {
  clearTimeout(settle);
  settle = setTimeout(() => video && send(video, false, true), QUEUE_SETTLE_MS);
});
const watchQueue = () => {
  const queue = document.querySelector(SELECTORS.queue);
  if (!queue || queue === watched) return;
  queueObserver.disconnect();
  queueObserver.observe(queue, { childList: true, subtree: true });
  watched = queue;
};

const onMedia = (e) => {
  if (!(e.target instanceof HTMLVideoElement)) return;
  video = e.target;
  if (e.type === 'loadedmetadata') watchQueue();
  send(video, e.type === 'seeked');
};

// Media events do not bubble, but they reach a capturing listener on the document, so this works
// for whatever <video> the page creates and needs no polling.
for (const type of ['play', 'pause', 'timeupdate', 'loadedmetadata', 'durationchange', 'seeked']) {
  document.addEventListener(type, onMedia, true);
}
