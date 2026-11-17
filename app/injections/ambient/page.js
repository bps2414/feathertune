// Ambient mode: a blurred, glowing copy of the cover behind the site, plus a halo of it right behind the
// cover on the player page so the picture melts into the background (style.css draws both). The
// background is a soft field of the cover's colors that Rust makes (ambient.rs) and sends as data, a
// small PNG in a `data:` URL, so it needs no blur. The halo is Rust's too: the cover already blurred and
// faded out toward its sides (a PNG with alpha), so the page runs no filter or mask for it. Which cover
// plays is read from the site's own media session metadata (only to know when there is one); nothing is
// fetched. Presentation only: it never touches the player or the ads. The drift (style.css) pauses while
// the window is hidden or minimized (page visibility) or in the background (no focus). Rust sends
// `{ off, field, halo }`. Plugin `ambient_mode`.
const HOSTS = ['googleusercontent.com', 'ggpht.com', 'ytimg.com'];
// The halo is this much bigger than the cover it sits behind.
const HALO_SCALE = 1.7;
const FIELD = /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/;
const MAX_FIELD = 100000;

const wrap = document.createElement('div');
wrap.id = 'feathertune-ambient';
wrap.setAttribute('aria-hidden', 'true');

// Two layers that cross-fade on a track change.
const stack = (className) => {
  const el = document.createElement('div');
  el.className = className;
  const layers = [document.createElement('div'), document.createElement('div')];
  for (const layer of layers) layer.className = 'ft-layer';
  el.append(...layers);
  return { el, layers, front: 0 };
};
const background = stack('ft-bg');
const halo = stack('ft-halo');
const scrim = document.createElement('div');
scrim.className = 'ft-scrim';
wrap.append(background.el, scrim, halo.el);
let shown = null;
let field = null; // the background from Rust
let glow = null; // the halo's picture from Rust

const cover = () => {
  const art = navigator.mediaSession?.metadata?.artwork;
  const src = art?.length ? art[art.length - 1].src : null;
  try {
    const url = new URL(src);
    const ok = url.protocol === 'https:' && HOSTS.some((h) => url.hostname === h || url.hostname.endsWith('.' + h));
    return ok ? url.href : null;
  } catch {
    return null;
  }
};

const show = (s, href) => {
  const next = s.layers[1 - s.front];
  next.style.backgroundImage = `url(${JSON.stringify(href)})`;
  next.classList.add('ft-show');
  s.layers[s.front].classList.remove('ft-show');
  s.front = 1 - s.front;
};

// The halo follows the picture on the player page and is off wherever there is none.
const place = () => {
  const r = document.querySelector('ytmusic-player#player')?.getBoundingClientRect();
  const on = !!r && r.width > 40 && r.height > 40 && !!shown;
  halo.el.classList.toggle('ft-on', on);
  if (!on) return;
  const w = r.width * HALO_SCALE;
  const h = r.height * HALO_SCALE;
  halo.el.style.cssText = `left:${r.left + r.width / 2 - w / 2}px;top:${r.top + r.height / 2 - h / 2}px;width:${w}px;height:${h}px`;
};

// Game mode (Rust: `set({ off })`) takes the layers out, so nothing is blurred or drawn, and puts them
// back with the current cover when it ends.
let off = false;

const update = () => {
  if (off) return;
  const href = cover();
  if (href !== shown) {
    shown = href;
    if (!wrap.isConnected && document.body) document.body.prepend(wrap);
    document.documentElement.toggleAttribute('data-feathertune-ambient', !!href);
  }
  place();
};

// The background follows Rust's field and the halo Rust's glow (none: only the dark base shows).
const paintOne = (s, picture) => {
  if (picture) show(s, picture);
  else for (const layer of s.layers) layer.classList.remove('ft-show');
};
const paint = () => {
  paintOne(background, field);
  paintOne(halo, glow);
};

const motion = () => {
  wrap.classList.toggle('ft-still', document.hidden || !document.hasFocus());
};

// The picture moves with the window size and the page layout; both are rare events.
let watched = null;
const resizes = new ResizeObserver(place);
const watch = () => {
  const player = document.querySelector('ytmusic-player#player');
  if (player && player !== watched) {
    resizes.disconnect();
    resizes.observe(player);
    watched = player;
  }
};

for (const type of ['loadedmetadata', 'play']) {
  document.addEventListener(
    type,
    () => {
      watch();
      update();
    },
    true,
  );
}
// In a gapless queue the next track starts on the same <video> without those events: its cover is
// checked on the time events (a string compare until it changes).
document.addEventListener(
  'timeupdate',
  () => {
    if (!off && cover() !== shown) {
      watch();
      update();
    }
  },
  true,
);
window.addEventListener('resize', () => off || place());
document.addEventListener('visibilitychange', motion);
window.addEventListener('focus', motion);
window.addEventListener('blur', motion);
motion();

feathertune.ambient = Object.freeze({
  set(data) {
    const next = data?.off === true;
    const picture = (given) => (typeof given === 'string' && given.length <= MAX_FIELD && FIELD.test(given) ? given : null);
    const f = picture(data?.field);
    const g = picture(data?.halo);
    if (f !== field) {
      field = f;
      if (!off && !next) paintOne(background, field);
    }
    if (g !== glow) {
      glow = g;
      if (!off && !next) paintOne(halo, glow);
    }
    if (next === off) return;
    off = next;
    if (off) {
      resizes.disconnect();
      watched = null;
      wrap.remove();
      document.documentElement.removeAttribute('data-feathertune-ambient');
      shown = null;
    } else {
      watch();
      update();
      paint();
    }
  },
});
