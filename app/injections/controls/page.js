// Player controls used by the tray menu. Every player selector the app depends on lives here,
// so a UI change on the site is fixed in this file only.
const SELECTORS = {
  playPause: 'ytmusic-player-bar #play-pause-button',
  next: 'ytmusic-player-bar .next-button',
  previous: 'ytmusic-player-bar .previous-button',
};

const click = (name) => {
  const el = document.querySelector(SELECTORS[name]);
  if (el) el.click();
  return !!el;
};

// Play and pause are separate keys on the media flyout; they only toggle when needed.
const paused = () => document.querySelector('video')?.paused ?? true;

feathertune.controls = Object.freeze({
  playPause: () => click('playPause'),
  play: () => paused() && click('playPause'),
  pause: () => !paused() && click('playPause'),
  next: () => click('next'),
  previous: () => click('previous'),
});
