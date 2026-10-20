// Ambient mode: a blurred, glowing copy of the cover behind the site, plus a halo of it right behind the
// cover on the player page so the picture melts into the background (style.css draws both). The cover
// is the one in the site's own media session metadata, the URL the player bar already shows, so it is
// already loaded; nothing else is fetched. Presentation only: it never touches the player or the ads.
// The drift (style.css) pauses while the window is hidden or minimized (page visibility) or in the
// background (no focus). Plugin `ambient_mode`.
const HOSTS = ['googleusercontent.com', 'ggpht.com', 'ytimg.com'];
// The halo is this much bigger than the cover it sits behind.
const HALO_SCALE = 1.7;

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

const update = () => {
  const href = cover();
  if (href !== shown) {
    shown = href;
    if (!wrap.isConnected && document.body) document.body.prepend(wrap);
    document.documentElement.toggleAttribute('data-feathertune-ambient', !!href);
    if (href) {
      show(background, href);
      show(halo, href);
    }
  }
  place();
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
window.addEventListener('resize', place);
document.addEventListener('visibilitychange', motion);
window.addEventListener('focus', motion);
window.addEventListener('blur', motion);
motion();
