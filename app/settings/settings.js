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
    if (t.dataset.tab === 'experimental') loadMix();
  });
}
// After an update the app opens the window on What's new (`#whats-new`).
if (tabs.some((t) => t.dataset.tab === location.hash.slice(1))) showTab(location.hash.slice(1));

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

// Mix: through settings_mix (mixes.rs). Track titles come from the site's queue and are shown with
// textContent only.
const CURVE_NAMES = { equal_power: 'Equal power', linear: 'Linear', s_curve: 'S-curve', exponential: 'Exponential' };
const EFFECT_NAMES = {
  echo: 'Echo on the outgoing tail',
  filter: 'High-pass rising on the outgoing track',
  bass_swap: 'Bass swap (needs the overlap)',
  tempo: 'Next track at this tempo (needs the overlap)',
};
const MIX_PRESETS = {
  fade: { duration_ms: 6000, curve: 'equal_power', effects: {} },
  rise: { duration_ms: 8000, curve: 's_curve', effects: { filter: true } },
  bass_swap: { duration_ms: 8000, curve: 'equal_power', effects: { bass_swap: true } },
  echo_out: { duration_ms: 4000, curve: 'exponential', effects: { echo: true } },
};
// Where the editor's canvas looks: the last and first seconds of the two tracks.
const MIX_VIEW_S = 40;
let mix = null;
let mixList = null;
// The pair being edited: { from, to, out_ms, in_ms, duration_ms, curve, effects }, its tracks' facts.
let editing = null;
let editFacts = { a: null, b: null };
const mixCall = async (op, arg = {}) => {
  try {
    const r = await invoke('settings_mix', { op, arg: { list: mixList, ...arg } });
    showError('');
    return r ?? true;
  } catch (e) {
    showError(String(e));
    return null;
  }
};
const seconds = (ms) => `${(ms / 1000).toFixed(1)} s`;
const time = (ms) => {
  const s = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
};
// Camelot neighbours mix well: the same code, one step round the wheel, or the same number.
const keysMatch = (a, b) => {
  if (!a || !b) return null;
  const [na, la] = [parseInt(a, 10), a.slice(-1)];
  const [nb, lb] = [parseInt(b, 10), b.slice(-1)];
  const step = Math.min((na - nb + 12) % 12, (nb - na + 12) % 12);
  return (la === lb && step <= 1) || (na === nb && la !== lb);
};

// A transition's fields (length, curve, effects); `onChange` gets the new transition.
const transitionFields = (box, t, max, onChange) => {
  const length = el('input');
  length.type = 'range';
  length.min = '0';
  length.max = String(max);
  length.step = '500';
  length.value = String(t.duration_ms);
  const shown = el('span', 'value', seconds(t.duration_ms));
  const lengthLabel = el('label', '', 'Length ');
  lengthLabel.append(shown, length);
  const curve = el('select');
  for (const [id, name] of Object.entries(CURVE_NAMES)) {
    const o = el('option', '', name);
    o.value = id;
    curve.append(o);
  }
  curve.value = t.curve;
  const curveLabel = el('label', '', 'Curve ');
  curveLabel.append(curve);
  const effects = Object.entries(EFFECT_NAMES).map(([id, name]) => {
    const row = el('div', 'option');
    const body = el('div', 'body');
    body.append(el('div', 'name small', name));
    const toggle = el('input', 'switch');
    toggle.type = 'checkbox';
    toggle.checked = !!t.effects?.[id];
    toggle.setAttribute('aria-label', name);
    toggle.dataset.effect = id;
    row.append(body, toggle);
    return row;
  });
  const read = () => ({
    ...t,
    duration_ms: Number(length.value),
    curve: curve.value,
    effects: Object.fromEntries(effects.map((r) => r.querySelector('input')).map((i) => [i.dataset.effect, i.checked])),
  });
  length.addEventListener('input', () => (shown.textContent = seconds(Number(length.value))));
  for (const input of [length, curve, ...effects.map((r) => r.querySelector('input'))]) {
    input.addEventListener('change', () => onChange(read()));
  }
  box.replaceChildren(lengthLabel, curveLabel, ...effects);
};

const renderMix = (m) => {
  mix = m;
  mixList = m.list;
  $('mix-none').hidden = !!m.list;
  $('mix-body').hidden = !m.list;
  const select = $('mix-list');
  const lists = [...new Set([...(m.list ? [m.list] : []), ...m.lists])];
  select.replaceChildren(
    ...lists.map((l) => {
      const o = el('option', '', l === m.current ? `${l} (playing)` : l);
      o.value = l;
      return o;
    }),
  );
  select.value = m.list || '';
  if (!m.list) return;
  $('mix-enabled').checked = m.mix.enabled;
  $('mix-auto').checked = m.mix.defaults.auto;
  $('mix-defaults').hidden = m.mix.defaults.auto;
  transitionFields($('mix-defaults'), m.mix.defaults, m.max_ms, async (t) => {
    if (await mixCall('defaults', { defaults: t })) loadMix();
  });
  const edited = new Map(m.mix.pairs.map((p) => [`${p.from}>${p.to}`, p]));
  const items = m.queue;
  $('mix-pairs').replaceChildren(
    ...items.slice(0, -1).map((a, i) => {
      const b = items[i + 1];
      const li = el('li', edited.has(`${a.id}>${b.id}`) ? 'edited' : '');
      const text = el('div', 'body');
      text.append(el('div', 'title', `${a.title || a.id} → ${b.title || b.id}`));
      const detail = [a.artist && b.artist ? `${a.artist} → ${b.artist}` : '', a.bpm ? `${Math.round(a.bpm)} BPM` : '', a.camelot && b.camelot ? `${a.camelot} → ${b.camelot}` : '']
        .filter(Boolean)
        .join(' · ');
      if (detail) text.append(el('div', 'detail', detail));
      const button = el('button', 'secondary', edited.has(`${a.id}>${b.id}`) ? 'Edited' : 'Auto');
      button.type = 'button';
      button.addEventListener('click', () => editPair(a, b));
      li.append(text, button);
      return li;
    }),
  );
  if (items.length < 2) $('mix-pairs').append(el('li', 'empty', 'Play this playlist once to list its tracks.'));
  if (m.open && m.open.list === m.list) {
    const a = items.find((x) => x.id === m.open.from) || { id: m.open.from, title: '' };
    const b = items.find((x) => x.id === m.open.to) || { id: m.open.to, title: '' };
    showTab('experimental');
    editPair(a, b);
  } else if (editing) {
    $('mix-editor').hidden = false;
  }
};

const editPair = async (a, b) => {
  const pair = mix.mix.pairs.find((p) => p.from === a.id && p.to === b.id);
  const [fa, fb] = await Promise.all([mixCall('facts', { id: a.id }), mixCall('facts', { id: b.id })]);
  editFacts = { a: fa && fa !== true ? fa : null, b: fb && fb !== true ? fb : null };
  const base = mix.mix.defaults;
  editing = pair
    ? { ...pair, effects: { ...pair.effects } }
    : { from: a.id, to: b.id, out_ms: null, in_ms: Math.round(editFacts.b?.lead_in_ms || 0), duration_ms: base.duration_ms, curve: base.curve, effects: { ...base.effects } };
  $('mix-title').textContent = `${a.title || a.id} → ${b.title || b.id}`;
  const match = keysMatch(editFacts.a?.camelot, editFacts.b?.camelot);
  const tempo = [editFacts.a?.bpm, editFacts.b?.bpm].map((x) => (x ? `${Math.round(x)} BPM` : '?')).join(' → ');
  $('mix-keys').textContent = [
    `Tempo ${tempo}`,
    match === null ? 'Keys not measured yet' : `Keys ${editFacts.a.key} → ${editFacts.b.key}: ${match ? 'they mix well' : 'they may clash'}`,
  ].join(' · ');
  $('mix-delete').disabled = !pair;
  $('mix-editor').hidden = false;
  drawPairFields();
  drawMixCanvas();
  $('mix-editor').scrollIntoView({ block: 'nearest' });
};
const drawPairFields = () =>
  transitionFields($('mix-pair-fields'), editing, mix.max_ms, (t) => {
    editing = t;
    drawMixCanvas();
  });

// The end of A on the top lane and the start of B below, from their envelopes (loudness, 10 a
// second); the fade's span and curve over A, B's entry point. Drawn only when something changes.
const outMs = () => {
  const end = editFacts.a ? editFacts.a.tail_ms ?? editFacts.a.duration_s * 1000 : null;
  return editing.out_ms ?? (end !== null ? Math.max(0, end - editing.duration_ms) : null);
};
const envelope = (facts) => {
  if (!facts?.envelope) return null;
  const raw = atob(facts.envelope);
  const out = new Float32Array(raw.length / 3);
  for (let i = 0; i < out.length; i++) out[i] = Math.max(raw.charCodeAt(i * 3), raw.charCodeAt(i * 3 + 1), raw.charCodeAt(i * 3 + 2)) / 255;
  return out;
};
const drawMixCanvas = () => {
  const canvas = $('mix-canvas');
  const g = canvas.getContext('2d');
  const { width: w, height: h } = canvas;
  const lane = h / 2;
  const css = getComputedStyle(document.body);
  const ink = css.color;
  g.clearRect(0, 0, w, h);
  g.fillStyle = ink;
  g.strokeStyle = ink;
  const view = MIX_VIEW_S * 1000;
  const aEnd = editFacts.a ? editFacts.a.duration_s * 1000 : null;
  const aStart = aEnd !== null ? Math.max(0, aEnd - view) : 0;
  const lanes = [
    { env: envelope(editFacts.a), from: aStart, y: 0 },
    { env: envelope(editFacts.b), from: 0, y: lane },
  ];
  g.globalAlpha = 0.35;
  for (const { env, from, y } of lanes) {
    if (!env) continue;
    for (let x = 0; x < w; x++) {
      const v = env[Math.floor((from + (x / w) * view) / 100)] ?? 0;
      g.fillRect(x, y + lane - 4 - v * (lane - 8), 1, v * (lane - 8));
    }
  }
  g.globalAlpha = 1;
  const out = outMs();
  if (out !== null) {
    const x0 = ((out - aStart) / view) * w;
    const x1 = ((out + editing.duration_ms - aStart) / view) * w;
    g.globalAlpha = 0.12;
    g.fillRect(x0, 0, x1 - x0, lane);
    g.globalAlpha = 1;
    g.beginPath();
    const curve = editing.curve;
    for (let i = 0; i <= 32; i++) {
      const t = i / 32;
      const v = curve === 'linear' ? 1 - t : curve === 's_curve' ? 0.5 * (1 + Math.cos(Math.PI * t)) : curve === 'exponential' ? (10 ** (-3 * t) - 0.001) / 0.999 : Math.cos((t * Math.PI) / 2);
      g.lineTo(x0 + t * (x1 - x0), 4 + (1 - v) * (lane - 8));
    }
    g.stroke();
  }
  const xin = (editing.in_ms / view) * w;
  g.fillRect(xin, lane, 2, lane);
  g.globalAlpha = 0.5;
  g.fillRect(0, lane, w, 1);
  g.globalAlpha = 1;
  g.font = '11px system-ui, sans-serif';
  g.fillText(out !== null ? `out ${time(out)}` : 'out: end of the track', 6, 12);
  g.fillText(`in ${time(editing.in_ms)}`, 6, lane + 12);
};
// Top lane: the fade's start; bottom lane: where B starts.
const dragMix = (e) => {
  if (!editing || !(e.buttons & 1)) return;
  const canvas = $('mix-canvas');
  const r = canvas.getBoundingClientRect();
  const x = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  const view = MIX_VIEW_S * 1000;
  if (e.clientY - r.top < r.height / 2) {
    if (!editFacts.a) return;
    const aEnd = editFacts.a.duration_s * 1000;
    const start = Math.max(0, aEnd - view);
    editing.out_ms = Math.round(Math.min(aEnd - editing.duration_ms, start + x * view));
    if (editing.out_ms < 0) editing.out_ms = 0;
  } else {
    editing.in_ms = Math.round(x * view);
  }
  drawMixCanvas();
};
$('mix-canvas').addEventListener('pointerdown', (e) => {
  e.currentTarget.setPointerCapture(e.pointerId);
  dragMix(e);
});
$('mix-canvas').addEventListener('pointermove', dragMix);

for (const b of document.querySelectorAll('#mix-presets button')) {
  b.addEventListener('click', async () => {
    const id = b.dataset.preset;
    if (id === 'auto') {
      if (mix.mix.pairs.some((p) => p.from === editing.from && p.to === editing.to)) {
        if (await mixCall('delete_pair', { from: editing.from, to: editing.to })) loadMix();
      }
      $('mix-editor').hidden = true;
      editing = null;
      return;
    }
    editing = { ...editing, ...MIX_PRESETS[id], effects: { ...MIX_PRESETS[id].effects } };
    drawPairFields();
    drawMixCanvas();
  });
}
$('mix-save').addEventListener('click', async () => {
  if (editing && (await mixCall('save_pair', { pair: editing }))) loadMix();
});
$('mix-preview').addEventListener('click', () => editing && mixCall('preview', { pair: editing }));
$('mix-delete').addEventListener('click', async () => {
  if (editing && (await mixCall('delete_pair', { from: editing.from, to: editing.to }))) {
    $('mix-editor').hidden = true;
    editing = null;
    loadMix();
  }
});
$('mix-list').addEventListener('change', () => {
  editing = null;
  $('mix-editor').hidden = true;
  loadMix($('mix-list').value);
});
$('mix-enabled').addEventListener('change', async () => {
  if (await mixCall('enable', { on: $('mix-enabled').checked })) loadMix();
});
$('mix-auto').addEventListener('change', async () => {
  if (await mixCall('defaults', { defaults: { ...mix.mix.defaults, auto: $('mix-auto').checked } })) loadMix();
});
$('mix-export').addEventListener('click', () => mixCall('export'));
$('mix-import').addEventListener('click', async () => {
  if (!confirm("Replace this playlist's mix with the pasted one?")) return;
  if (await mixCall('import', { json: $('mix-import-text').value })) {
    $('mix-import-text').value = '';
    loadMix();
  }
});

const loadMix = async (list) => {
  try {
    renderMix(await invoke('settings_mix', { op: 'get', arg: list || mixList ? { list: list || mixList } : {} }));
  } catch (e) {
    showError(String(e));
  }
};

// What's new: CHANGELOG.md compiled into the app, shown with textContent only. Releases newer than
// the version that ran before this update (`since`; "" when it is not known) are marked.
const newer = (a, b) => {
  const [x, y] = [a, b].map((v) => v.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
};
// `**bold**` and `[text](url)` (shown as its text); everything else is plain text.
const inline = (parent, text) => {
  for (const part of text.split(/(\*\*[^*]+\*\*|\[[^\]]+\]\([^)]*\))/)) {
    if (part.startsWith('**') && part.endsWith('**')) parent.append(el('strong', '', part.slice(2, -2)));
    else if (part.startsWith('[')) parent.append(part.slice(1, part.indexOf(']')));
    else if (part) parent.append(part);
  }
  return parent;
};
const renderChangelog = (text, since) => {
  const box = $('changelog');
  if (box.childElementCount) return;
  let card = null;
  let list = null;
  let first = true;
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith('## ')) {
      const version = line.slice(3).split(' ')[0];
      const isNew = since !== null && (since ? newer(version, since) : first);
      first = false;
      card = el('div', isNew ? 'card release new' : 'card release');
      const name = inline(el('div', 'name'), line.slice(3));
      if (isNew) name.append(el('span', 'badge', 'new'));
      card.append(name);
      box.append(card);
      list = null;
    } else if (!card || !line.trim()) {
      list = null;
    } else if (line.startsWith('### ')) {
      card.append(inline(el('h3'), line.slice(4)));
      list = null;
    } else if (line.startsWith('- ')) {
      if (!list) card.append((list = el('ul')));
      list.append(inline(el('li'), line.slice(2)));
    } else {
      card.append(inline(el('p'), line));
    }
  }
};

const load = async () => {
  try {
    const s = await invoke('settings_get');
    applyTheme(s.theme);
    renderChangelog(s.changelog, s.updated_from);
    renderPlugins(s.plugins);
    renderLastFm(s.lastfm);
    renderDesktopLyrics(s.desktop_lyrics);
    renderAudio(s.audio);
    // The Mix tab, and a pair the site asked to open (mix_open).
    loadMix();
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
      if (input.type === 'range') showSeconds(input);
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

// Range fields in ms show their value in seconds.
function showSeconds(input) {
  const out = input.parentElement.querySelector('[data-seconds]');
  if (out) out.textContent = `${(Number(input.value) / 1000).toFixed(1)} s`;
}
for (const input of document.querySelectorAll('input[type="range"][data-key]')) {
  input.addEventListener('input', () => showSeconds(input));
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
