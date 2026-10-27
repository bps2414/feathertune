// Player controls used by the tray menu, the media keys and the phone remote. Every player selector
// the app depends on lives here, so a UI change on the site is fixed in this file only. Seek, volume,
// the queue actions and opening a playlist are refused (false) while an ad plays.
const SELECTORS = {
  playPause: 'ytmusic-player-bar #play-pause-button',
  next: 'ytmusic-player-bar .next-button',
  previous: 'ytmusic-player-bar .previous-button',
  player: '#movie_player',
  video: 'video',
  // The rows of the queue, and the link in one when the row's data has no video id.
  queueRows: 'ytmusic-player-queue ytmusic-player-queue-item',
  rowLink: 'a[href*="watch?v="]',
  // A queue row's menu button and drag handle, and the entries of the open menu.
  rowMenu: 'ytmusic-menu-renderer yt-button-shape button, ytmusic-menu-renderer #button',
  dragHandle: '#drag-handle, .drag-handle, [class*="drag-handle"]',
  menuEntries: 'ytmusic-menu-popup-renderer ytmusic-menu-service-item-renderer',
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
const findRow = (id, index) => {
  if (typeof id !== 'string' || !/^[A-Za-z0-9_-]{11}$/.test(id)) return null;
  if (!Number.isInteger(index) || index < 0 || index >= MAX_QUEUE || adShowing()) return null;
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
  return row;
};
const playItem = (id, index) => {
  const row = findRow(id, index);
  if (!row) return false;
  row.click();
  return true;
};

// The row's own menu, then the entry of `action` in it: 'next' (play next) or 'remove' (from the queue).
// Entries are told apart by their endpoint or icon in the site's data, not by their text (any language).
const MENU_ENTRY = {
  next: (d) =>
    d?.serviceEndpoint?.queueAddEndpoint?.queueInsertPosition === 'INSERT_AFTER_CURRENT_VIDEO' ||
    d?.icon?.iconType === 'QUEUE_PLAY_NEXT',
  remove: (d) => !!d?.serviceEndpoint?.removeFromQueueEndpoint || d?.icon?.iconType === 'REMOVE',
};
const MENU_WAIT_MS = 1500;
const menuAction = (id, index, action) => {
  const row = findRow(id, index);
  const button = row?.querySelector(SELECTORS.rowMenu);
  if (!button) return false;
  button.click();
  const started = performance.now();
  const pick = () => {
    const entry = [...document.querySelectorAll(SELECTORS.menuEntries)].find(
      (e) => e.offsetParent !== null && MENU_ENTRY[action](e.data),
    );
    if (entry) return entry.click();
    if (performance.now() - started < MENU_WAIT_MS) return setTimeout(pick, 50);
    // Not in this menu: close it again.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  };
  setTimeout(pick, 50);
  return true;
};

// Moves queue row `index` (track `id`) to `to`. Right after the playing track is the menu's "play next";
// anywhere else the row's drag handle is dragged there with mouse events (the site may ignore them: then the
// phone sees the queue unchanged and falls back to its menu).
const DRAG_STEPS = 8;
const queueMove = (id, index, to) => {
  if (!Number.isInteger(to) || to < 0 || to >= MAX_QUEUE || to === index) return false;
  const row = findRow(id, index);
  if (!row) return false;
  const rows = [...document.querySelectorAll(SELECTORS.queueRows)];
  const current = rows.findIndex((r) => r.hasAttribute('selected') || r.getAttribute('play-button-state') === 'playing');
  if (current >= 0 && to === current + (index < current ? 0 : 1)) return menuAction(id, index, 'next');
  const target = rows[Math.min(to, rows.length - 1)];
  const handle = row.querySelector(SELECTORS.dragHandle) || row;
  if (!target) return false;
  const from = handle.getBoundingClientRect();
  const dest = target.getBoundingClientRect();
  const x = from.left + from.width / 2;
  const y0 = from.top + from.height / 2;
  // Past the target's middle, so it lands on the side the phone asked for.
  const y1 = to > index ? dest.bottom - 2 : dest.top + 2;
  const fire = (el, type, y, Kind = MouseEvent) =>
    el.dispatchEvent(
      new Kind(type, { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, buttons: 1, pointerId: 1, isPrimary: true }),
    );
  fire(handle, 'pointerdown', y0, PointerEvent);
  fire(handle, 'mousedown', y0);
  for (let i = 1; i <= DRAG_STEPS; i++) {
    const y = y0 + ((y1 - y0) * i) / DRAG_STEPS;
    const over = document.elementFromPoint(x, y) || target;
    fire(over, 'pointermove', y, PointerEvent);
    fire(over, 'mousemove', y);
  }
  const end = document.elementFromPoint(x, y1) || target;
  fire(end, 'pointerup', y1, PointerEvent);
  fire(end, 'mouseup', y1);
  return true;
};

// Plays playlist `list` of the library: the site's own page for it, which starts it.
const openPlaylist = (list) => {
  if (typeof list !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(list) || adShowing()) return false;
  location.assign(`/watch?list=${list}`);
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
  queueNext: (id, index) => menuAction(id, index, 'next'),
  queueRemove: (id, index) => menuAction(id, index, 'remove'),
  queueMove,
  openPlaylist,
});
