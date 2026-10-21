// Song mode: when a track comes in a song and a video version, press the site's own "Song" button of its
// Song / Video switch once per track, so the video is not decoded. It is the click the user could make; the
// switch stays visible, no attribute of the site is rewritten, and a click on "Video" by the user holds
// until the next track. Nothing happens during an ad or before the player exists.
if (window.top !== window) return;

const SELECTORS = {
  player: '#movie_player',
  page: 'ytmusic-player-page',
  mode: 'ytmusic-player',
  songButton: 'ytmusic-av-toggle .song-button',
  videoButton: 'ytmusic-av-toggle .video-button',
};

// The id of the track the choice was made for (or that the user chose Video for).
let handled = null;
let observer = null;

const adShowing = (player) => player.classList.contains('ad-showing') || player.classList.contains('ad-interrupting');
const trackId = (player) =>
  player.getVideoData?.().video_id || new URLSearchParams(location.search).get('v');

const act = () => {
  const player = document.querySelector(SELECTORS.player);
  if (!player || adShowing(player)) return;
  const id = trackId(player);
  if (!id || id === handled) return;
  const page = document.querySelector(SELECTORS.page);
  const song = document.querySelector(SELECTORS.songButton);
  const video = document.querySelector(SELECTORS.videoButton);
  // Tracks without a video version have no switch: nothing to do (and nothing recorded, it may still appear).
  if (!page?.hasAttribute('has-av-switcher') || !song || !video) return;
  const onVideo =
    video.getAttribute('aria-pressed') === 'true' ||
    document.querySelector(SELECTORS.mode)?.getAttribute('playback-mode') === 'OMV_PREFERRED';
  handled = id;
  if (onVideo) song.click();
};

// The switch is drawn a moment after the track starts: watch the two attributes that tell it appeared
// or changed, only on the elements that carry them.
const watch = () => {
  if (observer) return;
  const page = document.querySelector(SELECTORS.page);
  const mode = document.querySelector(SELECTORS.mode);
  if (!page || !mode) return;
  observer = new MutationObserver(act);
  observer.observe(page, { attributes: true, attributeFilter: ['has-av-switcher'] });
  observer.observe(mode, { attributes: true, attributeFilter: ['playback-mode'] });
};

const onMedia = (e) => {
  if (!(e.target instanceof HTMLVideoElement)) return;
  if (!document.querySelector(SELECTORS.player)) {
    observer?.disconnect();
    observer = null;
    return;
  }
  watch();
  act();
};

// Media events do not bubble, but they reach a capturing listener on the document.
for (const type of ['loadedmetadata', 'playing']) document.addEventListener(type, onMedia, true);

// The user chose Video: leave it for the rest of this track. Our own click is not trusted.
document.addEventListener(
  'click',
  (e) => {
    if (!e.isTrusted || !e.target.closest?.(SELECTORS.videoButton)) return;
    const player = document.querySelector(SELECTORS.player);
    if (player) handled = trackId(player);
  },
  true,
);

feathertune.song_mode = Object.freeze({});
