// Album color. Rust sends { accent, ink, onAccent } (hex strings) once per cover, or null when there is
// no cover or the plugin's data is cleared; this turns them into CSS variables on <html>, which style.css
// uses. Presentation only: nothing here touches the player or the ads. Data comes from Rust only.
const HEX = /^#[0-9a-f]{6}$/i;
const VARS = { accent: '--feathertune-accent', ink: '--feathertune-ink', onAccent: '--feathertune-on-accent' };

feathertune.theme = Object.freeze({
  set(data) {
    const root = document.documentElement;
    const valid = !!data && Object.keys(VARS).every((key) => typeof data[key] === 'string' && HEX.test(data[key]));
    for (const [key, name] of Object.entries(VARS)) {
      if (valid) root.style.setProperty(name, data[key]);
      else root.style.removeProperty(name);
    }
    root.toggleAttribute('data-feathertune-theme', valid);
  },
});
