// The settings window. Talks to Rust only through the settings_* commands, which only this window
// may call (capabilities/settings-window.json). Every change is saved as soon as it is made.
'use strict';

const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const el = (tag, className, text) => {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
};

const showError = (message) => {
  $('error').textContent = message || '';
  $('error').hidden = !message;
};

const set = async (key, value, input) => {
  try {
    const restart = await invoke('settings_set', { key, value });
    input?.classList.remove('invalid');
    showError('');
    if (restart) $('restart').hidden = false;
    return true;
  } catch (e) {
    input?.classList.add('invalid');
    showError(String(e));
    return false;
  }
};

// A text field that saved: a short green edge.
const flashSaved = (input) => {
  input.classList.add('saved');
  setTimeout(() => input.classList.remove('saved'), 900);
};

// Tabs.
const tabs = [...document.querySelectorAll('[role="tab"]')];
const panels = [...document.querySelectorAll('[role="tabpanel"]')];
let currentTab = 'general';
const showTab = (name) => {
  currentTab = name;
  for (const t of tabs) t.setAttribute('aria-selected', String(t.dataset.tab === name));
  for (const p of panels) p.hidden = p.dataset.panel !== name;
  if (name === 'stats') loadStats();
};
for (const t of tabs) {
  t.addEventListener('click', () => {
    clearSearch();
    showTab(t.dataset.tab);
  });
}

// Search: the cards of every tab whose text has the query; empty, back to the current tab.
const search = () => {
  const q = $('search').value.trim().toLowerCase();
  document.body.classList.toggle('searching', !!q);
  if (!q) {
    for (const c of document.querySelectorAll('main .card')) c.hidden = false;
    $('no-results').hidden = true;
    showTab(currentTab);
    return;
  }
  let any = false;
  for (const p of panels) {
    let shown = false;
    for (const c of p.querySelectorAll('.card')) {
      const match = c.textContent.toLowerCase().includes(q);
      c.hidden = !match;
      shown ||= match;
    }
    p.hidden = !shown;
    any ||= shown;
  }
  $('no-results').hidden = any;
};
const clearSearch = () => {
  if (!$('search').value) return;
  $('search').value = '';
  search();
};
$('search').addEventListener('input', search);

// The album colors of the title bar (null: the default look of the CSS).
const applyTheme = (theme) => {
  const root = document.documentElement.style;
  for (const name of ['--ink', '--g1', '--g2', '--g3', '--g4']) root.removeProperty(name);
  if (!theme) return;
  root.setProperty('--ink', theme.ink);
  theme.glows.forEach((c, i) => root.setProperty(`--g${i + 1}`, c));
};

// Plugin cards: each `[data-plugin]` card gets the plugin's switch, name, description and cost on top,
// with its own fields below. Plugins without a card go to Advanced.
const switches = new Map();
const buildPlugins = (plugins) => {
  for (const p of plugins) {
    let card = document.querySelector(`[data-plugin="${p.id}"]`);
    if (!card) {
      card = el('div', 'card');
      card.dataset.plugin = p.id;
      $('other-plugins').append(card);
    }
    const extra = el('div', 'extra');
    extra.append(...card.childNodes);
    const name = el('div', 'name', p.name);
    if (!p.live) name.append(el('span', 'badge', 'restart'));
    const body = el('div', 'body');
    const cost = el('details', 'cost');
    cost.append(el('summary', '', 'Cost'), document.createTextNode(p.cost));
    body.append(name, el('div', 'description', p.description), cost);
    const toggle = el('input', 'switch');
    toggle.type = 'checkbox';
    toggle.setAttribute('aria-label', p.name);
    toggle.addEventListener('change', async () => {
      if (!(await set(`plugins.${p.id}`, toggle.checked, null))) toggle.checked = !toggle.checked;
      card.classList.toggle('off', !toggle.checked);
    });
    const row = el('div', 'row');
    row.append(body, toggle);
    card.replaceChildren(row, extra);
    switches.set(p.id, toggle);
  }
};

const renderPlugins = (plugins) => {
  if (!switches.size) buildPlugins(plugins);
  for (const p of plugins) {
    const toggle = switches.get(p.id);
    toggle.checked = p.on;
    toggle.closest('.card').classList.toggle('off', !p.on);
  }
};

const renderLastFm = (l) => {
  const button = $('lastfm-button');
  button.hidden = l.connected === null;
  $('lastfm-dot').classList.toggle('on', l.connected === true);
  if (l.connected === null) {
    $('lastfm-status').textContent = 'Not running: turn the plugin on (with API keys) and restart.';
  } else {
    $('lastfm-status').textContent = l.connected ? 'Connected' : 'Not connected';
    button.textContent = l.connected ? 'Disconnect' : 'Connect';
    button.className = l.connected ? 'secondary' : '';
    button.dataset.action = l.connected ? 'lastfm_disconnect' : 'lastfm_connect';
  }
  $('lastfm-keys').textContent = l.built_in
    ? 'This build has its own API keys; keys typed here take their place.'
    : 'This build has no API keys of its own: add yours (from last.fm/api).';
};

// Lyrics sources: the enabled ones in their order, then the others; a switch and up/down for each.
// Every change saves the enabled ones, in order.
let sources = [];
const saveSources = async () => {
  const before = sources;
  const ids = sources.filter((x) => x.on).map((x) => x.id);
  if (!(await set('lyrics.providers', ids, null))) sources = before;
  drawSources();
};
const drawSources = () => {
  $('lyrics-providers').replaceChildren(
    ...sources.map((src, i) => {
      const li = el('li', src.on ? '' : 'off');
      const toggle = el('input', 'switch');
      toggle.type = 'checkbox';
      toggle.checked = src.on;
      toggle.setAttribute('aria-label', src.name);
      toggle.addEventListener('change', () => {
        sources = sources.map((x) => (x === src ? { ...x, on: toggle.checked } : x));
        saveSources();
      });
      const move = (by, label) => {
        const b = el('button', 'secondary', label === 'up' ? '↑' : '↓');
        b.type = 'button';
        b.setAttribute('aria-label', `Move ${src.name} ${label}`);
        b.disabled = i + by < 0 || i + by >= sources.length;
        b.addEventListener('click', () => {
          const next = [...sources];
          [next[i], next[i + by]] = [next[i + by], next[i]];
          sources = next;
          saveSources();
        });
        return b;
      };
      li.append(el('span', 'label', src.name), move(-1, 'up'), move(1, 'down'), toggle);
      return li;
    }),
  );
};
const renderSources = (l) => {
  const name = (id) => l.all_providers.find((p) => p.id === id)?.name;
  const on = l.providers.filter(name).map((id) => ({ id, name: name(id), on: true }));
  const off = l.all_providers.filter((p) => !l.providers.includes(p.id)).map((p) => ({ ...p, on: false }));
  sources = [...on, ...off];
  drawSources();
};

const renderDesktopLyrics = (d) => {
  $('desktop-lyrics').textContent = d.open ? 'Hide floating lyrics' : 'Show floating lyrics';
  $('desktop-lyrics-lock').hidden = !d.open;
  $('desktop-lyrics-lock').textContent = d.locked ? 'Unlock to move' : 'Lock in place';
};

// Stats: everything comes from settings_stats as numbers and names, shown with textContent only.
let statsRange = 'month';
let statsRunning = true;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const duration = (seconds) => {
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h ? `${h} h ${m} min` : `${m} min`;
};
const tile = (value, label) => {
  const t = el('div', 'tile');
  t.append(el('div', 'value', value), el('div', 'label', label));
  return t;
};
const renderTop = (list, items) => {
  const max = items[0]?.seconds || 1;
  list.replaceChildren(
    ...items.map((item) => {
      const li = el('li');
      li.style.setProperty('--share', String(item.seconds / max));
      const text = el('div', 'body');
      text.append(el('div', 'title', item.name));
      if (item.detail) text.append(el('div', 'detail', item.detail));
      li.append(text, el('div', 'amount', `${duration(item.seconds)} · ${item.plays} plays`));
      return li;
    }),
  );
  if (!items.length) list.append(el('li', 'empty', 'Nothing yet.'));
};
// A point of the series: a day (days since 1970, local) or a month.
const pointLabel = (p) => (p.day !== undefined ? String(new Date(p.day * 86400000).getUTCDate()) : MONTHS[p.month - 1]);
const pointTitle = (p) =>
  p.day !== undefined
    ? new Date(p.day * 86400000).toLocaleDateString(undefined, { timeZone: 'UTC' })
    : `${MONTHS[p.month - 1]} ${p.year}`;
const renderStats = (s) => {
  $('stats-tiles').replaceChildren(
    tile(duration(s.seconds), 'listened'),
    tile(String(s.plays), `plays (${s.skips} skipped)`),
    tile(String(s.artists), s.new_artists ? `artists, ${s.new_artists} new` : 'artists'),
    tile(`${s.streak} d`, `streak (best ${s.longest_streak} d)`),
  );
  const max = Math.max(1, ...s.series.map((p) => p.seconds));
  $('stats-series').replaceChildren(
    ...s.series.map((p) => {
      const bar = el('div', 'bar');
      bar.title = `${pointTitle(p)}: ${duration(p.seconds)}`;
      const fill = el('i');
      fill.style.height = `${(p.seconds / max) * 100}%`;
      bar.append(fill, el('span', '', pointLabel(p)));
      return bar;
    }),
  );
  const hottest = Math.max(1, ...s.heat.flat());
  const heat = $('stats-heat');
  heat.replaceChildren();
  s.heat.forEach((hours, d) => {
    heat.append(el('span', 'day', DAYS[d]));
    hours.forEach((seconds, h) => {
      const cell = el('i');
      cell.style.opacity = String(0.08 + 0.92 * (seconds / hottest));
      cell.title = `${DAYS[d]} ${h}:00: ${duration(seconds)}`;
      heat.append(cell);
    });
  });
  renderTop($('stats-artists'), s.top_artists);
  renderTop($('stats-tracks'), s.top_tracks);
  renderTop($('stats-albums'), s.top_albums);
};
const loadStats = async () => {
  $('stats-off').hidden = statsRunning;
  $('stats-body').hidden = !statsRunning;
  if (!statsRunning) return;
  try {
    renderStats(await invoke('settings_stats', { range: statsRange, offset: new Date().getTimezoneOffset() }));
    showError('');
  } catch (e) {
    showError(String(e));
  }
};
for (const b of document.querySelectorAll('#stats-range button')) {
  b.addEventListener('click', () => {
    statsRange = b.dataset.range;
    for (const o of document.querySelectorAll('#stats-range button')) o.setAttribute('aria-pressed', String(o === b));
    loadStats();
  });
}

// Audio: the equalizer's presets and its 10 bands (a band moved by hand switches to Custom).
const EQ_LABELS = ['31', '62', '125', '250', '500', '1k', '2k', '4k', '8k', '16k'];
const PRESET_NAMES = { flat: 'Flat', bass: 'Bass', vocal: 'Vocal', treble: 'Treble', loudness: 'Loudness', custom: 'Custom' };
const bands = [];
const dbText = (v) => `${v > 0 ? '+' : ''}${v}`;
const saveBands = async () => {
  const values = bands.map((b) => Number(b.input.value));
  if (!(await set('audio.eq_bands', values, null))) return;
  if ($('eq-preset').value !== 'custom') {
    $('eq-preset').value = 'custom';
    await set('audio.eq_preset', 'custom', null);
  }
};
const renderAudio = (a) => {
  const select = $('eq-preset');
  if (!select.options.length) {
    for (const id of [...a.eq_presets, 'custom']) {
      const o = el('option', '', PRESET_NAMES[id] || id);
      o.value = id;
      select.append(o);
    }
  }
  if (!bands.length) {
    EQ_LABELS.forEach((label, i) => {
      const input = el('input');
      input.type = 'range';
      input.min = '-12';
      input.max = '12';
      input.step = '0.5';
      input.setAttribute('aria-label', `${label} Hz`);
      const db = el('span', 'db');
      input.addEventListener('input', () => (db.textContent = dbText(Number(input.value))));
      input.addEventListener('change', saveBands);
      const band = el('div', 'band');
      band.append(db, input, el('span', '', label));
      $('eq-bands').append(band);
      bands[i] = { input, db };
    });
  }
  a.eq_bands.forEach((v, i) => {
    if (bands[i].input === document.activeElement) return;
    bands[i].input.value = String(v);
    bands[i].db.textContent = dbText(v);
  });
};
// A preset chosen from the list moves the bands to it.
$('eq-preset').addEventListener('change', () => setTimeout(load, 100));

const load = async () => {
  try {
    const s = await invoke('settings_get');
    applyTheme(s.theme);
    renderPlugins(s.plugins);
    renderLastFm(s.lastfm);
    renderDesktopLyrics(s.desktop_lyrics);
    renderAudio(s.audio);
    if (!$('lyrics-providers').contains(document.activeElement)) renderSources(s.lyrics);
    statsRunning = s.stats_running;
    if (!document.querySelector('[data-panel="stats"]').hidden) loadStats();
    $('restart').hidden = !s.restart_needed;
    for (const input of document.querySelectorAll('[data-key]')) {
      if (input === document.activeElement) continue;
      const value = input.dataset.key.split('.').reduce((o, k) => o?.[k], s);
      if (input.type === 'checkbox') input.checked = !!value;
      else input.value = Array.isArray(value) ? value.join(', ') : (value ?? '');
      if ('shortcut' in input.dataset) input.placeholder = 'Off';
    }
  } catch (e) {
    showError(String(e));
  }
};

for (const input of document.querySelectorAll('[data-key]:not([data-shortcut])')) {
  input.addEventListener('change', async () => {
    let value = input.type === 'checkbox' ? input.checked : input.value;
    if ('list' in input.dataset) value = value.split(',').map((x) => x.trim()).filter(Boolean);
    if ((await set(input.dataset.key, value, input)) && input.type !== 'checkbox') flashSaved(input);
  });
}

// Shortcut fields record the keys pressed, in the names the shortcut parser takes (Ctrl+Alt+P).
const keyName = (code) => {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);
  if (code.startsWith('Arrow')) return code.slice(5);
  return code;
};
const MODIFIERS = ['Control', 'Alt', 'Shift', 'Meta'];
for (const input of document.querySelectorAll('[data-shortcut]')) {
  let before = '';
  const stop = (value) => {
    input.classList.remove('recording');
    input.value = value;
    input.blur();
  };
  input.addEventListener('focus', () => {
    before = input.value;
    input.classList.add('recording');
    input.value = 'Press keys…';
  });
  input.addEventListener('blur', () => {
    if (input.classList.contains('recording')) stop(before);
  });
  input.addEventListener('keydown', async (e) => {
    e.preventDefault();
    if (e.key === 'Escape') return stop(before);
    if (e.key === 'Backspace' || e.key === 'Delete') {
      stop('');
      if (!(await set(input.dataset.key, '', input))) input.value = before;
      return;
    }
    if (MODIFIERS.includes(e.key)) return;
    const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean);
    if (!mods.length) {
      input.value = 'Add Ctrl, Alt, Shift or Win';
      return;
    }
    const keys = [...mods, keyName(e.code)].join('+');
    stop(keys);
    if (await set(input.dataset.key, keys, input)) flashSaved(input);
    else input.value = before;
  });
}

const action = async (name) => {
  try {
    await invoke('settings_action', { action: name });
    showError('');
  } catch (err) {
    showError(String(err));
  }
};

$('lastfm-button').addEventListener('click', async (e) => {
  await action(e.currentTarget.dataset.action);
  setTimeout(load, 500);
});
$('mini-player').addEventListener('click', () => action('mini_player'));
$('share-lyrics').addEventListener('click', () => action('share_lyrics'));
for (const [id, name] of [
  ['desktop-lyrics', 'desktop_lyrics'],
  ['desktop-lyrics-lock', 'desktop_lyrics_lock'],
]) {
  $(id).addEventListener('click', async () => {
    await action(name);
    setTimeout(load, 200);
  });
}
$('stats-export').addEventListener('click', () => action('stats_export'));
$('stats-clear').addEventListener('click', async () => {
  if (!confirm('Delete the whole listening history? This cannot be undone.')) return;
  await action('stats_clear');
  loadStats();
});
$('restart-button').addEventListener('click', () => action('restart'));

// The Last.fm sign-in finishes in the browser, and the track (its colors) may change: refresh when
// the window comes back.
window.addEventListener('focus', load);
load();
