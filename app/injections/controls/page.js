// Player controls used by the tray menu, the media keys and the phone remote. Every player selector
// the app depends on lives here, so a UI change on the site is fixed in this file only. Seek, volume
// and play-a-queue-row are refused (false) while an ad plays.
const SELECTORS = {
  playPause: 'ytmusic-player-bar #play-pause-button',
  next: 'ytmusic-player-bar .next-button',
  previous: 'ytmusic-player-bar .previous-button',
  player: '#movie_player',
  video: 'video',
  // The rows of the queue, and the link in one when the row's data has no video id.
  queueRows: 'ytmusic-player-queue ytmusic-player-queue-item',
  rowLink: 'a[href*="watch?v="]',
};
// Volume keys without the precise_volume module: one 5% step through the player's public API.
const VOLUME_STEP = 5;
// The queue is read up to 200 tracks (core), so no row after that is played.
const MAX_QUEUE = 200;

const click = (name) => {
  const el = document.querySelector(SELECTORS[name]);
  if (el) el.click();
  return !!el;
};

// Play and pause are separate keys on the media flyout; they only toggle when needed.
const paused = () => document.querySelector(SELECTORS.video)?.paused ?? true;

const adShowing = () => {
  const player = document.querySelector(SELECTORS.player);
  return !!player && (player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting'));
};

// The global volume keys. precise_volume does it when it is loaded (2% and the percentage on screen).
const volume = (direction) => {
  if (feathertune.precise_volume) return feathertune.precise_volume.step(direction);
  const player = document.querySelector(SELECTORS.player);
  if (!player?.getVolume || !player.setVolume) return false;
  if (adShowing()) return false;
  if (direction > 0 && player.isMuted?.()) player.unMute?.();
  player.setVolume(Math.max(0, Math.min(100, player.getVolume() + Math.sign(direction) * VOLUME_STEP)));
  return true;
};

// The remote's volume slider: an absolute value, 0-100.
const setVolume = (value) => {
  if (!Number.isFinite(value) || value < 0 || value > 100) return false;
  if (feathertune.precise_volume) return feathertune.precise_volume.set(value);
  const player = document.querySelector(SELECTORS.player);
  if (!player?.getVolume || !player.setVolume) return false;
  if (adShowing()) return false;
  if (value > 0 && player.isMuted?.()) player.unMute?.();
  player.setVolume(Math.round(value));
  return true;
};

// Moves the track to `seconds` into it (on the track's clock, so a gapless queue is right), at most to
// its end.
const seek = (seconds) => {
  const video = document.querySelector(SELECTORS.video);
  if (!video || !Number.isFinite(seconds) || seconds < 0 || adShowing()) return false;
  const length = feathertune.clock.duration(video);
  feathertune.clock.seek(video, Number.isFinite(length) && length > 0 ? Math.min(seconds, length) : seconds);
  return true;
};

const rowId = (row) => {
  const id = row.data?.videoId;
  if (typeof id === 'string') return id;
  const href = row.querySelector(SELECTORS.rowLink)?.getAttribute('href') || '';
  return new URLSearchParams(href.split('?')[1] || '').get('v');
};

// Plays the track `id` at queue row `index`; only a row of that track is ever clicked (a stale index never
// plays another song). `index` counts the queue entries core could read, and the page may have more rows
// than that (a hidden song/video counterpart), so when the row there is another track, the row of `id`
// closest to `index` is played.
const playItem = (id, index) => {
  if (typeof id !== 'string' || !/^[A-Za-z0-9_-]{11}$/.test(id)) return false;
  if (!Number.isInteger(index) || index < 0 || index >= MAX_QUEUE || adShowing()) return false;
  const rows = [...document.querySelectorAll(SELECTORS.queueRows)];
  let row = rows[index] && rowId(rows[index]) === id ? rows[index] : null;
  if (!row) {
    let best = Infinity;
    rows.forEach((r, i) => {
      if (Math.abs(i - index) < best && rowId(r) === id) {
        best = Math.abs(i - index);
        row = r;
      }
    });
  }
  if (!row) return false;
  row.click();
  return true;
};

feathertune.controls = Object.freeze({
  playPause: () => click('playPause'),
  play: () => paused() && click('playPause'),
  pause: () => !paused() && click('playPause'),
  next: () => click('next'),
  previous: () => click('previous'),
  volumeUp: () => volume(1),
  volumeDown: () => volume(-1),
  seek,
  setVolume,
  playItem,
});
