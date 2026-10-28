// The host of the imported extensions (extensions.rs). Loaded when any extension with page code runs; each one
// is its own initialization script that asks `feathertune.ext.context(meta)` for its API object (`ft`), the
// page side of docs/EXTENSIONS.md. Rust gives the host data as `feathertune.ext.set({ settings: { id: {...} } })`.
// Extensions are the user's own code: nothing here limits what they do with the page.
const contexts = new Map();
// The ids that should run: one that never asked for its context did not start (a syntax error, most likely).
const expected = new Set(EXPECTED);
// At most 20 reports per extension per page load reach Rust (the Extensions tab lists the last 20).
const reports = new Map();
const call = (id, op, arg) => window.__TAURI_INTERNALS__.invoke('ext_call', { id, op, arg: arg ?? null });
const report = (id, message) => {
  console.error(`[feathertune] extension ${id}:`, message);
  const n = (reports.get(id) || 0) + 1;
  reports.set(id, n);
  if (n <= 20) call(id, 'error', { message: String(message).slice(0, 2000) }).catch(() => {});
};
const text = (e) => String(e?.stack || e?.message || e).slice(0, 500);

const context = (meta) => {
  const id = meta.id;
  let settings = Object.freeze({ ...meta.settings });
  const settingsListeners = new Set();
  const ft = {
    api: 1,
    id,
    version: meta.version,
    permissions: Object.freeze([...meta.permissions]),
    get settings() {
      return settings;
    },
    onSettings(fn) {
      settingsListeners.add(fn);
      return () => settingsListeners.delete(fn);
    },
    addStyle(css) {
      const s = document.createElement('style');
      s.dataset.feathertune = `ext:${id}`;
      s.textContent = String(css);
      const add = () => (document.head || document.documentElement).append(s);
      document.documentElement ? add() : document.addEventListener('readystatechange', add, { once: true });
      return s;
    },
    log(...args) {
      console.log(`[${id}]`, ...args);
    },
  };
  contexts.set(id, {
    ft,
    setSettings(next) {
      settings = Object.freeze({ ...next });
      for (const fn of settingsListeners) {
        try {
          fn(settings);
        } catch (e) {
          report(id, text(e));
        }
      }
    },
  });
  expected.delete(id);
  return ft;
};

window.addEventListener(
  'load',
  () => {
    for (const id of expected) report(id, 'did not start (a syntax error in page.js?)');
  },
  { once: true },
);

feathertune.ext = Object.freeze({
  context,
  failed: (id, e) => report(id, text(e)),
  set(data) {
    for (const [id, values] of Object.entries(data?.settings || {})) contexts.get(id)?.setSettings(values);
  },
});
