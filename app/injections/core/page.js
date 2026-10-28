// Player state bridge (always on). Reads navigator.mediaSession.metadata and the <video> and
// sends them to Rust through the `player_state` command: right away on a track, play/pause or
// seek change, plus a position refresh every 15 s while playing (Rust extrapolates in between).
// Each IPC call is a fetch with a CORS preflight: sending every second cost ~3% CPU (measured).
// With them go the player's volume, mute and ad flag (a volume or ad change is sent right away, from the
// video's volumechange and an observer on the player's classes), and where the track plays from: the
// playlist (`list=` of the URL, for the mixes), the next track and, when it changed, the queue (at most
// 200 tracks, with or without a playlist: radio and autoplay too) and the playing track's index in it,
// read from the site's state on a track change and when the queue's DOM changes (an observer, no polling).
// The playlists of the library (id and title, as the sidebar lists them, at most 100) go alone, for the phone
// remote: when the sidebar's DOM changes (an observer), so also before anything plays.
// One-way: Rust never sends anything back through here.
// The site plays a queue back to back on one <video> (gapless): the next track is appended after the
// one playing, so the video's time and duration go on counting from the first track. The player's
// public API has the track's own; `feathertune.clock` (for every module) turns the video's time into
// the track's. The offset only changes at a track change, so it moves only by more than a second (the
// player's time may be a moment behind the video's).
let trackOffset = 0;
const offsetOf = (video) => {
  try {
    const t = document.querySelector('#movie_player')?.getCurrentTime?.();
    if (Number.isFinite(t) && Number.isFinite(video.currentTime)) {
      const o = Math.max(0, video.currentTime - t);
      if (Math.abs(o - trackOffset) > 1) trackOffset = o < 1 ? 0 : o;
    }
  } catch {}
  return trackOffset;
};
feathertune.clock = Object.freeze({
  // The track's time (s) at the video's time now.
  time: (video) => video.currentTime - offsetOf(video),
  // The track's length (s): the player's, else the video's (no queue appended after it).
  duration: (video) => {
    try {
      const d = document.querySelector('#movie_player')?.getDuration?.();
      if (Number.isFinite(d) && d > 0) return d;
    } catch {}
    return video.duration;
  },
  // Moves the video to `t` s into the track.
  seek: (video, t) => {
    video.currentTime = t + offsetOf(video);
  },
});
if (window.top !== window) return;

const SELECTORS = {
  // Its classes say when an ad plays.
  player: '#movie_player',
  // Its getState() is the site's state: `queue.items`, `selectedItemIndex`, `automixItems`, `repeatMode`.
  playerBar: 'ytmusic-player-bar',
  queue: '#queue',
  // The sidebar, and its entries (a playlist's `data` browses to `VL<list>`; else its link has `list=`).
  guide: 'ytmusic-guide-renderer',
  guideEntries: 'ytmusic-guide-entry-renderer',
  guideLink: 'a[href*="list="], a[href*="browse/VL"]',
};
const MAX_LIBRARY = 100;
const LIBRARY_SETTLE_MS = 1000;
// Tries to find the sidebar after load (it renders late); no polling once it is found or after the last try.
const GUIDE_TRIES_MS = [1000, 3000, 8000, 20000];
const REFRESH_MS = 15000;
const MAX_QUEUE = 200;
const QUEUE_SETTLE_MS = 500;
const VOLUME_SETTLE_MS = 150;
let lastKey = '';
let lastSent = 0;
let lastQueue = '';
let lastWhere = '';
let lastAd = false;
let video = null;

const adShowing = () => {
  const p = document.querySelector(SELECTORS.player);
  return !!p && (p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting'));
};

// The player's own volume (0-100) and mute, through its public API; the video's when that is missing.
const volumeOf = (video) => {
  let p = null;
  try {
    p = document.querySelector(SELECTORS.player);
  } catch {}
  const v = p?.getVolume?.();
  const volume = Number.isFinite(v) ? v : video.volume * 100;
  const muted = typeof p?.isMuted === 'function' ? !!p.isMuted() : video.muted;
  return { volume: Math.max(0, Math.min(100, Math.round(volume))), muted };
};

// The playing video's id: the player's (the URL only has `v=` on the player page), else the URL's.
const videoId = () => {
  let id = null;
  try {
    id = document.querySelector(SELECTORS.player)?.getVideoData?.()?.video_id;
  } catch {}
  return typeof id === 'string' && /^[A-Za-z0-9_-]{11}$/.test(id) ? id : new URLSearchParams(location.search).get('v');
};

// The track's length: the player's. While the player has none (between tracks) it is 0 (unknown) and not
// the <video>'s, which in a gapless queue counts the tracks appended after it. Without the player's API
// (the site changed) the <video>'s is all there is.
const lengthOf = (video) => {
  try {
    const p = document.querySelector(SELECTORS.player);
    if (typeof p?.getDuration === 'function') {
      const d = p.getDuration();
      return Number.isFinite(d) && d > 0 ? d : 0;
    }
  } catch {}
  return video.duration;
};

// The playing queue entry's own length ("3:56" is 236), read with the queue in `where`. The player's
// getDuration sometimes answers only what is loaded so far (20 s, then 38 s, then 50 s...), which makes the
// lyrics sources miss the track; when the two are far apart for the same video, the queue's wins.
let queueLength = { id: null, secs: 0 };
const lengthSeconds = (r) => {
  const t = r?.lengthText?.simpleText ?? (r?.lengthText?.runs || []).map((x) => x.text).join('');
  const parts = String(t || '').split(':').map(Number);
  if (parts.length > 3 || parts.some((n) => !Number.isInteger(n) || n < 0)) return 0;
  return parts.reduce((total, n) => total * 60 + n, 0);
};
const trusted = (secs, id) =>
  secs > 0 && queueLength.id === id && queueLength.secs > 0 && Math.abs(secs - queueLength.secs) > 3 ? queueLength.secs : secs;

// While an ad plays the <video> is the ad's, so its duration is not the track's: it goes as 0 (unknown).
const read = (video) => {
  const m = navigator.mediaSession?.metadata;
  if (!m || !m.title) return null;
  const art = m.artwork?.length ? m.artwork[m.artwork.length - 1].src : null;
  const id = videoId();
  const num = (x) => (Number.isFinite(x) ? Math.round(x * 10) / 10 : 0);
  return {
    ...volumeOf(video),
    ad: adShowing(),
    id,
    title: m.title,
    artist: m.artist || '',
    album: m.album || '',
    artwork: art,
    duration: adShowing() ? 0 : num(trusted(lengthOf(video), id)),
    position: num(feathertune.clock.time(video)),
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
// autoplay's first), the queue and the playing track's index in it (`current`, null when unknown).
const where = () => {
  const list = new URLSearchParams(location.search).get('list');
  const out = { list: list && /^[A-Za-z0-9_-]{1,64}$/.test(list) ? list : null, next: null, queue: [], current: null };
  let q = null;
  try {
    q = document.querySelector(SELECTORS.playerBar)?.getState?.()?.queue;
  } catch {}
  if (!q || !Array.isArray(q.items)) return out;
  const items = q.items.map(item);
  out.queue = items.filter(Boolean).slice(0, MAX_QUEUE);
  const i = q.selectedItemIndex;
  if (Number.isInteger(i)) {
    const r = q.items[i]?.playlistPanelVideoRenderer || q.items[i]?.playlistPanelVideoWrapperRenderer?.primaryRenderer?.playlistPanelVideoRenderer;
    queueLength = { id: r?.videoId ?? null, secs: lengthSeconds(r) };
    // An index into the list without the entries that could not be read.
    if (items[i]) {
      const at = items.slice(0, i).filter(Boolean).length;
      if (at < out.queue.length) out.current = at;
    }
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
  lastAd = s.ad;
  const key = [s.id, s.title, s.artist, s.album, s.artwork, s.duration, s.paused, s.volume, s.muted, s.ad].join('\u0000');
  const now = performance.now();
  const due = force || (!s.paused && now - lastSent >= REFRESH_MS);
  if (key === lastKey && !due && !queueChanged) return;
  // The site's state is only read here: on a change, not on every timeupdate.
  const w = where();
  s.duration = Math.round(trusted(s.duration, s.id) * 10) / 10;
  const whereKey = [w.list, w.next?.id, w.current].join('\u0000');
  if (key === lastKey && !due && whereKey === lastWhere && w.queue.map((x) => x.id).join(',') === lastQueue) return;
  lastKey = key;
  lastWhere = whereKey;
  lastSent = now;
  s.list = w.list;
  s.next = w.next;
  s.current = w.current;
  const queueKey = w.queue.map((x) => x.id).join(',');
  if (queueKey !== lastQueue) {
    lastQueue = queueKey;
    s.queue = w.queue;
  }
  window.__TAURI_INTERNALS__?.invoke('player_state', { state: JSON.stringify(s) }).catch(() => {});
  // The imported extensions' page code gets the same state (injections/extensions/page.js), when any runs.
  try {
    feathertune.ext?.state(s);
  } catch {}
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

// A sidebar entry as { list, title }, or null when it is not a playlist.
const playlist = (entry) => {
  const browse = entry.data?.navigationEndpoint?.browseEndpoint?.browseId;
  let list = typeof browse === 'string' && browse.startsWith('VL') ? browse.slice(2) : null;
  if (!list) {
    const href = entry.querySelector(SELECTORS.guideLink)?.getAttribute('href') || '';
    list = new URLSearchParams(href.split('?')[1] || '').get('list') || (href.match(/browse\/VL([A-Za-z0-9_-]+)/) || [])[1];
  }
  const runs = entry.data?.formattedTitle?.runs;
  const title = (Array.isArray(runs) ? runs.map((r) => r.text).join('') : entry.querySelector('.title')?.textContent || '')
    .trim()
    .slice(0, 200);
  return list && /^[A-Za-z0-9_-]{1,64}$/.test(list) && title ? { list, title } : null;
};
let lastLibrary = '';
const sendLibrary = () => {
  const seen = new Set();
  const library = [];
  for (const entry of document.querySelectorAll(`${SELECTORS.guide} ${SELECTORS.guideEntries}`)) {
    const p = playlist(entry);
    if (p && !seen.has(p.list) && library.length < MAX_LIBRARY) {
      seen.add(p.list);
      library.push(p);
    }
  }
  const key = JSON.stringify(library);
  if (key === lastLibrary) return;
  lastLibrary = key;
  window.__TAURI_INTERNALS__?.invoke('player_state', { state: JSON.stringify({ library }) }).catch(() => {});
};
let librarySettle = 0;
let watchedGuide = null;
const guideObserver = new MutationObserver(() => {
  clearTimeout(librarySettle);
  librarySettle = setTimeout(sendLibrary, LIBRARY_SETTLE_MS);
});
const watchGuide = () => {
  const guide = document.querySelector(SELECTORS.guide);
  if (!guide || guide === watchedGuide) return !!guide;
  guideObserver.disconnect();
  guideObserver.observe(guide, { childList: true, subtree: true });
  watchedGuide = guide;
  sendLibrary();
  return true;
};
for (const ms of GUIDE_TRIES_MS) setTimeout(() => watchedGuide?.isConnected || watchGuide(), ms);

// An ad starts or ends: the player's classes change (the media events may not say so by themselves).
const adObserver = new MutationObserver(() => {
  if (video && adShowing() !== lastAd) send(video, true);
});
let watchedPlayer = null;
const watchPlayer = () => {
  const player = document.querySelector(SELECTORS.player);
  if (!player || player === watchedPlayer) return;
  adObserver.disconnect();
  adObserver.observe(player, { attributes: true, attributeFilter: ['class'] });
  watchedPlayer = player;
};

// A volume slider drag fires many volumechange events: send the last one once it settles.
let volumeSettle = 0;
const onMedia = (e) => {
  if (!(e.target instanceof HTMLVideoElement)) return;
  video = e.target;
  if (e.type === 'loadedmetadata') {
    watchQueue();
    watchPlayer();
    watchGuide();
  }
  if (e.type === 'volumechange') {
    clearTimeout(volumeSettle);
    volumeSettle = setTimeout(() => send(video, false), VOLUME_SETTLE_MS);
    return;
  }
  send(video, e.type === 'seeked');
};

// Media events do not bubble, but they reach a capturing listener on the document, so this works
// for whatever <video> the page creates and needs no polling.
for (const type of ['play', 'pause', 'timeupdate', 'loadedmetadata', 'durationchange', 'seeked', 'volumechange']) {
  document.addEventListener(type, onMedia, true);
}
