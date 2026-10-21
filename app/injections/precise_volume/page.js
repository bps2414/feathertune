// Precise volume: the mouse wheel over the player bar or the expanded player changes the volume by 2% a
// step (10% with Shift), through the embedded player's public API (getVolume / setVolume / isMuted /
// unMute); the site's volume slider is moved to match and a small "NN%" fades in beside it. Without that
// API nothing happens and the wheel keeps its default. `video.volume` is never touched, and nothing
// changes during an ad. `feathertune.precise_volume.step` is what the global volume keys call.
if (window.top !== window) return;

const SELECTORS = {
  player: '#movie_player',
  wheelAreas: 'ytmusic-player-bar, #main-panel',
  slider: '#volume-slider',
  bar: 'ytmusic-player-bar',
};
const HUD_ID = 'feathertune-volume-hud';
const HUD_MS = 1000;
const STEP = 2;
const BIG_STEP = 10;
// One notch of a mouse wheel is a delta of 100 on Windows; a touchpad sends many small ones.
const NOTCH = 100;

let hud = null;
let hideTimer = 0;
let carry = 0;

const adShowing = (player) => player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting');
const usable = (player) =>
  !!player && typeof player.getVolume === 'function' && typeof player.setVolume === 'function' && !adShowing(player);

const show = (volume) => {
  if (!hud) {
    hud = document.createElement('div');
    hud.id = HUD_ID;
    hud.setAttribute('aria-hidden', 'true');
    document.body.append(hud);
  }
  hud.textContent = `${volume}%`;
  // Beside the site's slider, or above the right end of the player bar when it is hidden (narrow window).
  const slider = document.querySelector(SELECTORS.slider)?.getBoundingClientRect();
  if (slider?.width) {
    hud.style.left = `${slider.left + slider.width / 2}px`;
    hud.style.top = `${slider.top - 8}px`;
  } else {
    const bar = document.querySelector(SELECTORS.bar)?.getBoundingClientRect();
    hud.style.left = `${bar ? bar.right - 60 : 60}px`;
    hud.style.top = `${bar ? bar.top - 8 : 60}px`;
  }
  hud.classList.add('on');
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => hud.classList.remove('on'), HUD_MS);
};

// Moves the volume by `steps` percent (negative: down). Returns whether it did.
const step = (steps) => {
  const player = document.querySelector(SELECTORS.player);
  if (!usable(player)) return false;
  if (steps > 0 && player.isMuted?.()) player.unMute?.();
  const volume = Math.max(0, Math.min(100, Math.round(player.getVolume() + steps)));
  player.setVolume(volume);
  const slider = document.querySelector(SELECTORS.slider);
  if (slider) slider.value = volume;
  show(volume);
  return true;
};

document.addEventListener(
  'wheel',
  (e) => {
    if (e.ctrlKey || !e.target.closest?.(SELECTORS.wheelAreas)) return;
    if (!usable(document.querySelector(SELECTORS.player))) return;
    e.preventDefault();
    // With Shift held Chromium reports the wheel as horizontal.
    const delta = e.deltaY || (e.shiftKey ? e.deltaX : 0);
    if (carry * delta < 0) carry = 0;
    carry += delta * (e.deltaMode === 0 ? 1 : NOTCH);
    const notches = Math.trunc(carry / NOTCH);
    if (!notches) return;
    carry -= notches * NOTCH;
    step(-notches * (e.shiftKey ? BIG_STEP : STEP));
  },
  { passive: false },
);

feathertune.precise_volume = Object.freeze({
  step: (direction) => step(Math.sign(direction) * STEP),
});
