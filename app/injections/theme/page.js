// Album color. Rust sends { accent, ink, onAccent, glows } (hex strings; `glows` four of them) once per
// cover, or null when there is no cover or the plugin's data is cleared; this turns them into CSS
// variables on <html>, which style.css uses (the glows: `--feathertune-glow-1` to `-4`, for the lyrics'
// verses). Presentation only: nothing here touches the player or the ads. Data comes from Rust only.
const HEX = /^#[0-9a-f]{6}$/i;
const VARS = { accent: '--feathertune-accent', ink: '--feathertune-ink', onAccent: '--feathertune-on-accent' };
const GLOWS = 4;

feathertune.theme = Object.freeze({
  set(data) {
    const root = document.documentElement;
    const valid = !!data && Object.keys(VARS).every((key) => typeof data[key] === 'string' && HEX.test(data[key]));
    for (const [key, name] of Object.entries(VARS)) {
      if (valid) root.style.setProperty(name, data[key]);
      else root.style.removeProperty(name);
    }
    const glows =
      valid && Array.isArray(data.glows) && data.glows.length === GLOWS && data.glows.every((g) => HEX.test(g));
    for (let i = 0; i < GLOWS; i++) {
      if (glows) root.style.setProperty(`--feathertune-glow-${i + 1}`, data.glows[i]);
      else root.style.removeProperty(`--feathertune-glow-${i + 1}`);
    }
    root.toggleAttribute('data-feathertune-theme', valid);
  },
});
