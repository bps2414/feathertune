// Player state bridge (always on). Reads navigator.mediaSession.metadata and the <video> and
// sends them to Rust through the `player_state` command: right away on a track, play/pause or
// seek change, plus a position refresh every 15 s while playing (Rust extrapolates in between).
// Each IPC call is a fetch with a CORS preflight: sending every second cost ~3% CPU (measured).
// One-way: Rust never sends anything back through here.
if (window.top !== window) return;

const REFRESH_MS = 15000;
let lastKey = '';
let lastSent = 0;

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
    duration: num(video.duration),
    position: num(video.currentTime),
    paused: video.paused,
  };
};

const onMedia = (e) => {
  const video = e.target;
  if (!(video instanceof HTMLVideoElement)) return;
  const s = read(video);
  if (!s) return;
  const key = [s.id, s.title, s.artist, s.album, s.artwork, s.duration, s.paused].join('\u0000');
  const now = performance.now();
  const due = e.type === 'seeked' || (!s.paused && now - lastSent >= REFRESH_MS);
  if (key === lastKey && !due) return;
  lastKey = key;
  lastSent = now;
  window.__TAURI_INTERNALS__?.invoke('player_state', { state: JSON.stringify(s) }).catch(() => {});
};

// Media events do not bubble, but they reach a capturing listener on the document, so this works
// for whatever <video> the page creates and needs no polling.
for (const type of ['play', 'pause', 'timeupdate', 'loadedmetadata', 'durationchange', 'seeked']) {
  document.addEventListener(type, onMedia, true);
}
