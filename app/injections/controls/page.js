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
  // What a tap on a queue row plays: its play button over the thumbnail, else its title.
  rowPlay: 'ytmusic-play-button-renderer, .song-info',
  // A queue row's menu button, and the entries of the open menu.
  rowMenu: 'ytmusic-menu-renderer yt-button-shape button, ytmusic-menu-renderer #button',
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
const isPlaying = (row) => row.hasAttribute('selected') || row.getAttribute('play-button-state') === 'playing';

// A tap on `el` as the site's own handlers see one: pointer and mouse down and up at its middle, then the
// click (a queue row does not react to a bare click() on it).
const tap = (el) => {
  const r = el.getBoundingClientRect();
  const at = { bubbles: true, cancelable: true, composed: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 };
  const pointer = { ...at, pointerId: 1, isPrimary: true, pointerType: 'mouse' };
  el.dispatchEvent(new PointerEvent('pointerdown', { ...pointer, buttons: 1 }));
  el.dispatchEvent(new MouseEvent('mousedown', { ...at, buttons: 1 }));
  el.dispatchEvent(new PointerEvent('pointerup', pointer));
  el.dispatchEvent(new MouseEvent('mouseup', at));
  el.dispatchEvent(new MouseEvent('click', at));
};

const playItem = (id, index) => {
  const row = findRow(id, index);
  if (!row) return false;
  tap(row.querySelector(SELECTORS.rowPlay) || row);
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
// `done` runs once the entry was clicked.
const menuAction = (id, index, action, done) => {
  const row = findRow(id, index);
  const button = row?.querySelector(SELECTORS.rowMenu);
  if (!button) return false;
  button.click();
  const started = performance.now();
  const pick = () => {
    const entry = [...document.querySelectorAll(SELECTORS.menuEntries)].find(
      (e) => e.offsetParent !== null && MENU_ENTRY[action](e.data),
    );
    if (entry) {
      entry.click();
      return done?.();
    }
    if (performance.now() - started < MENU_WAIT_MS) return setTimeout(pick, 50);
    // Not in this menu: close it again.
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  };
  setTimeout(pick, 50);
  return true;
};

// Play next as a move: the site's own "Play next" adds a copy of the row after the playing one, so once the
// copy shows up the row it came from is removed through its own menu. A row already next stays put; when no
// copy shows up within COPY_WAIT_MS nothing is removed.
const COPY_WAIT_MS = 2000;
const queueNext = (id, index) => {
  const row = findRow(id, index);
  if (!row) return false;
  const rows = () => [...document.querySelectorAll(SELECTORS.queueRows)];
  const copies = () => rows().filter((r) => rowId(r) === id).length;
  const at = rows().indexOf(row);
  const current = rows().findIndex(isPlaying);
  if (current >= 0 && at === current + 1) return true;
  const before = copies();
  return menuAction(id, index, 'next', () => {
    const started = performance.now();
    const removeOriginal = () => {
      if (copies() <= before) {
        if (performance.now() - started < COPY_WAIT_MS) setTimeout(removeOriginal, 100);
        return;
      }
      // The copy is right after the playing row; the original is the other row of the track nearest its place.
      const next = rows().findIndex(isPlaying) + 1;
      let original = -1;
      rows().forEach((r, i) => {
        if (i !== next && rowId(r) === id && (original < 0 || Math.abs(i - at) < Math.abs(original - at))) original = i;
      });
      if (original >= 0) menuAction(id, original, 'remove');
    };
    setTimeout(removeOriginal, 100);
  });
};

// Our own navigation (openPlaylist) leaves the page: the site's beforeunload handler would stop it with a
// "Leave site?" dialog that nobody at the PC asked for. This module runs before the site's scripts, so this
// listener comes first and keeps the site's from running, only for that navigation.
let leaving = false;
window.addEventListener('beforeunload', (e) => leaving && e.stopImmediatePropagation(), { capture: true });

// Plays playlist `list` of the library: the site's own page for it, which starts it.
const openPlaylist = (list) => {
  if (typeof list !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(list) || adShowing()) return false;
  leaving = true;
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
  queueNext,
  queueRemove: (id, index) => menuAction(id, index, 'remove'),
  openPlaylist,
});
