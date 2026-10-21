// Skip disliked tracks: a track that STARTS already marked with the site's dislike is skipped with the
// player's own next button. Read about 1 s after the track starts, so a dislike the user presses during a
// track (which the site already handles) never triggers a second skip. Never during an ad; one skip per
// track, and at most 5 in a row until a track has played for 10 s (a queue of disliked tracks ends).
if (window.top !== window) return;

const SELECTORS = {
  player: '#movie_player',
  like: '#like-button-renderer',
  // The same as `next` in controls/page.js.
  next: 'ytmusic-player-bar .next-button',
};
const READ_DELAY_MS = 1000;
const PLAYED_MS = 10000;
const MAX_IN_A_ROW = 5;

let seen = null;
let skippedId = null;
let inARow = 0;
let timer = 0;

const adShowing = (player) => player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting');
const trackId = (player) =>
  player.getVideoData?.().video_id || new URLSearchParams(location.search).get('v');

const check = (id) => {
  const player = document.querySelector(SELECTORS.player);
  if (!player || adShowing(player) || trackId(player) !== id) return;
  if (document.querySelector(SELECTORS.like)?.getAttribute('like-status') !== 'DISLIKE') {
    // It plays: after 10 s the streak of skips is over.
    timer = setTimeout(() => {
      const video = document.querySelector('video');
      const now = document.querySelector(SELECTORS.player);
      if (now && trackId(now) === id && video && video.currentTime >= PLAYED_MS / 1000) inARow = 0;
    }, PLAYED_MS);
    return;
  }
  if (skippedId === id || inARow >= MAX_IN_A_ROW) return;
  skippedId = id;
  inARow++;
  document.querySelector(SELECTORS.next)?.click();
};

const onMedia = (e) => {
  if (!(e.target instanceof HTMLVideoElement)) return;
  const player = document.querySelector(SELECTORS.player);
  // During an ad nothing is recorded: the track is looked at when its own audio starts.
  if (!player || adShowing(player)) return;
  const id = trackId(player);
  if (!id || id === seen) return;
  seen = id;
  clearTimeout(timer);
  timer = setTimeout(() => check(id), READ_DELAY_MS);
};

for (const type of ['loadedmetadata', 'playing']) document.addEventListener(type, onMedia, true);

feathertune.skip_disliked = Object.freeze({});
