// The host of the imported extensions (extensions.rs). Loaded when any extension with page code runs; each one
// is its own initialization script that asks `feathertune.ext.context(meta)` for its API object (`ft`), the
// page side of docs/EXTENSIONS.md. Rust gives the host data as `set({ settings: { id: {...} } })`, the lyrics
// and album colors as `feed(kind, data)`, and a WebAssembly's messages as `native(id, data)`; core gives it the
// player state as `state(s)`. Extensions are the user's own code: nothing here limits what they do with the page.
const contexts = new Map();
// The ids that should run: one that never asked for its context did not start (a syntax error, most likely).
const expected = new Set(EXPECTED);
const call = (id, op, arg) => window.__TAURI_INTERNALS__.invoke('ext_call', { id, op, arg: arg ?? null });
// At most 20 reports per extension per page load reach Rust (the Extensions tab lists the last 20).
const reports = new Map();
const report = (id, message) => {
  console.error(`[feathertune] extension ${id}:`, message);
  const n = (reports.get(id) || 0) + 1;
  reports.set(id, n);
  if (n <= 20) call(id, 'error', { message: String(message).slice(0, 2000) }).catch(() => {});
};
const text = (e) => String(e?.stack || e?.message || e).slice(0, 2000);

// What the extensions read: the last player state core sent, the queue, the lyrics and the album colors.
let player = null;
let queue = { queue: [], current: null, next: null, list: null };
let lyrics = null;
let colors = null;
const EVENTS = ['state', 'track', 'play', 'pause', 'seek', 'ad', 'queue', 'lyrics', 'colors', 'settings', 'native'];

const emit = (type, data, only) => {
  for (const [id, c] of contexts) {
    if (only && id !== only) continue;
    for (const fn of c.listeners[type]) {
      try {
        fn(data);
      } catch (e) {
        report(id, text(e));
      }
    }
  }
};

const video = () => document.querySelector('video');
// The state with the position of now (core sends it on a change and every 15 s; the clock is exact).
const snapshot = () => {
  if (!player) return null;
  const v = video();
  const position = v && !player.ad ? Math.round(feathertune.clock.time(v) * 10) / 10 : player.position;
  return Object.freeze({ ...player, position, paused: v ? v.paused : player.paused });
};

const CONTROLS = ['play', 'pause', 'playPause', 'next', 'previous', 'seek', 'setVolume', 'volumeUp', 'volumeDown'];

const context = (meta) => {
  const id = meta.id;
  const has = (p) => meta.permissions.includes(p);
  const settings = Object.freeze({ ...meta.settings });
  const listeners = Object.fromEntries(EVENTS.map((t) => [t, new Set()]));
  const on = (type, fn) => {
    if (!listeners[type]) throw new Error(`unknown event ${type}`);
    listeners[type].add(fn);
    return () => listeners[type].delete(fn);
  };
  const controls = {};
  if (has('player.control')) {
    // Through the controls module (the player's own buttons and API); false when it is off or refused.
    for (const name of CONTROLS) controls[name] = (...args) => feathertune.controls?.[name]?.(...args) ?? false;
  }
  const ft = {
    api: 1,
    id,
    version: meta.version,
    permissions: Object.freeze([...meta.permissions]),
    get settings() {
      return settings;
    },
    on,
    onSettings: (fn) => on('settings', fn),
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
      call(id, 'log', { message: args.map((a) => (typeof a === 'string' ? a : JSON.stringify(a))).join(' ').slice(0, 2000) }).catch(
        () => {},
      );
    },
    error: (e) => report(id, text(e)),
    player: Object.freeze({
      state: snapshot,
      queue: () => queue,
      clock: feathertune.clock,
      video,
      ...controls,
    }),
    theme: Object.freeze({ colors: () => colors }),
    lyrics: Object.freeze({ current: () => lyrics }),
    storage: Object.freeze({
      get: (key) => call(id, 'storage_get', { key }),
      set: (key, value) => call(id, 'storage_set', { key, value }),
      remove: (key) => call(id, 'storage_remove', { key }),
      keys: () => call(id, 'storage_keys'),
    }),
    fetch: (url, options = {}) =>
      call(id, 'fetch', { url: String(url), method: options.method, headers: options.headers, body: options.body }),
    notify: (title, body) => call(id, 'notify', { title, body }),
    native: Object.freeze({
      available: !!meta.native,
      send: (data) => call(id, 'native', { data }),
    }),
  };
  contexts.set(id, { ft, listeners });
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

document.addEventListener(
  'seeked',
  (e) => {
    if (e.target instanceof HTMLVideoElement && player && !player.ad) emit('seek', snapshot());
  },
  true,
);

feathertune.ext = Object.freeze({
  context,
  failed: (id, e) => report(id, text(e)),
  set(data) {
    for (const [id, values] of Object.entries(data?.settings || {})) {
      const c = contexts.get(id);
      if (!c) continue;
      const next = Object.freeze({ ...values });
      if (JSON.stringify(next) === JSON.stringify(c.ft.settings)) continue;
      // The getter reads the new values from here on.
      Object.defineProperty(c.ft, 'settings', { get: () => next, configurable: true });
      emit('settings', next, id);
    }
  },
  state(s) {
    const before = player;
    const { queue: q, list, next, current, ...rest } = s;
    if (q || before?.list !== list || before?.current !== current) {
      queue = Object.freeze({ queue: q ?? queue.queue, current, next, list });
      emit('queue', queue);
    }
    player = { ...rest, list };
    const now = snapshot();
    emit('state', now);
    if (!before || before.id !== s.id || before.title !== s.title || before.artist !== s.artist) emit('track', now);
    if (!before || before.paused !== s.paused) emit(s.paused ? 'pause' : 'play', now);
    if (before && before.ad !== s.ad) emit('ad', s.ad);
  },
  feed(kind, data) {
    if (kind === 'lyrics') {
      lyrics = data;
      emit('lyrics', data);
    } else if (kind === 'theme') {
      colors = data;
      emit('colors', data);
    }
  },
  native(id, data) {
    emit('native', data, id);
  },
});
