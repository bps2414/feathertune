// Player controls used by the tray menu. Every player selector the app depends on lives here,
// so a UI change on the site is fixed in this file only.
const SELECTORS = {
  playPause: 'ytmusic-player-bar #play-pause-button',
  next: 'ytmusic-player-bar .next-button',
  previous: 'ytmusic-player-bar .previous-button',
  player: '#movie_player',
};
// Volume keys without the precise_volume module: one 5% step through the player's public API.
const VOLUME_STEP = 5;

const click = (name) => {
  const el = document.querySelector(SELECTORS[name]);
  if (el) el.click();
  return !!el;
};

// Play and pause are separate keys on the media flyout; they only toggle when needed.
const paused = () => document.querySelector('video')?.paused ?? true;

// The global volume keys. precise_volume does it when it is loaded (2% and the percentage on screen).
const volume = (direction) => {
  if (feathertune.precise_volume) return feathertune.precise_volume.step(direction);
  const player = document.querySelector(SELECTORS.player);
  if (!player?.getVolume || !player.setVolume) return false;
  if (player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting')) return false;
  if (direction > 0 && player.isMuted?.()) player.unMute?.();
  player.setVolume(Math.max(0, Math.min(100, player.getVolume() + Math.sign(direction) * VOLUME_STEP)));
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
});
