// A card with the title and artist, shown for `seconds` when the track changes. One timeout per track; nothing
// runs while the card is hidden.
ft.addStyle(`
  #ft-track-card {
    position: fixed; z-index: 9999; max-width: 320px; padding: 10px 14px; border-radius: 10px;
    background: rgba(20, 20, 20, 0.92); color: #fff; font: 13px/1.4 system-ui, sans-serif;
    border-left: 3px solid var(--ft-card-accent); pointer-events: none; opacity: 0; transition: opacity 0.2s;
  }
  #ft-track-card.shown { opacity: 1; }
  #ft-track-card b { display: block; font-size: 14px; }
`);

let card = null;
let timer = 0;

const place = () => {
  const [v, h] = ft.settings.corner.split('-');
  Object.assign(card.style, { top: '', bottom: '', left: '', right: '', [v]: '80px', [h]: '16px' });
  card.style.setProperty('--ft-card-accent', ft.settings.accent);
};

ft.on('track', (state) => {
  if (!state?.title || state.ad) return;
  if (!card) {
    card = document.createElement('div');
    card.id = 'ft-track-card';
    document.body.append(card);
  }
  place();
  card.replaceChildren(Object.assign(document.createElement('b'), { textContent: state.title }), state.artist);
  card.classList.add('shown');
  clearTimeout(timer);
  timer = setTimeout(() => card.classList.remove('shown'), ft.settings.seconds * 1000);
});

ft.onSettings(() => card && place());
