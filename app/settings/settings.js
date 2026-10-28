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

// The app's language: `strings` (English → translation, from settings_get; empty for English).
// `t` translates one text, `tf` fills its `{}` in order; the HTML's own text is translated once.
let strings = {};
const t = (s) => strings[s] ?? s;
const tf = (s, ...args) => args.reduce((out, a) => out.replace('{}', a), t(s));
const translatePage = () => {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.nodeValue.trim();
    if (text && strings[text]) n.nodeValue = n.nodeValue.replace(text, strings[text]);
  }
  for (const e of document.querySelectorAll('[placeholder], [aria-label], [title]')) {
    for (const name of ['placeholder', 'aria-label', 'title']) {
      const text = e.getAttribute(name);
      if (text && strings[text]) e.setAttribute(name, strings[text]);
    }
  }
  document.title = t(document.title);
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

// Remote section state (see "Remote" below); declared here because showTab and search use it.
let remote = null; // the last settings_remote `get`
let qr = null; // the QR on screen: { size, modules }
let pairUntil = 0; // when the open pairing window ends (Date.now() ms)
let pairTimer = 0;

// Tabs.
const tabs =[...document.querySelectorAll('[role="tab"]')];
const panels = [...document.querySelectorAll('[role="tabpanel"]')];
let currentTab = 'general';
const showTab = (name) => {
  currentTab = name;
  for (const t of tabs) t.setAttribute('aria-selected', String(t.dataset.tab === name));
  for (const p of panels) p.hidden = p.dataset.panel !== name;
  if (name === 'stats') loadStats();
  if (name === 'extensions') loadExtensions();
  syncPairing();
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
  syncPairing();
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
    if (!p.live) name.append(el('span', 'badge', t('restart')));
    const body = el('div', 'body');
    const cost = el('details', 'cost');
    cost.append(el('summary', '', t('Cost')), document.createTextNode(p.cost));
    body.append(name, el('div', 'description', p.description), cost);
    const toggle = el('input', 'switch');
    toggle.type = 'checkbox';
    toggle.setAttribute('aria-label', p.name);
    toggle.addEventListener('change', async () => {
      if (!(await set(`plugins.${p.id}`, toggle.checked, null))) toggle.checked = !toggle.checked;
      card.classList.toggle('off', !toggle.checked);
      if (p.id === 'remote') loadRemote();
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
    $('lastfm-status').textContent = t('Not running: turn the plugin on (with API keys) and restart.');
  } else {
    $('lastfm-status').textContent = t(l.connected ? 'Connected' : 'Not connected');
    button.textContent = t(l.connected ? 'Disconnect' : 'Connect');
    button.className = l.connected ? 'secondary' : '';
    button.dataset.action = l.connected ? 'lastfm_disconnect' : 'lastfm_connect';
  }
  $('lastfm-keys').textContent = t(
    l.built_in
      ? 'This build has its own API keys; keys typed here take their place.'
      : 'This build has no API keys of its own: add yours (from last.fm/api).',
  );
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
        b.setAttribute('aria-label', tf(label === 'up' ? 'Move {} up' : 'Move {} down', src.name));
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
  $('desktop-lyrics').textContent = t(d.open ? 'Hide floating lyrics' : 'Show floating lyrics');
  $('desktop-lyrics-lock').hidden = !d.open;
  $('desktop-lyrics-lock').textContent = t(d.locked ? 'Unlock to move' : 'Lock in place');
};

// Stats: everything comes from settings_stats as numbers and names, shown with textContent only.
let statsRange = 'month';
let statsRunning = true;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const month = (m) => t(MONTHS[m - 1]);
const duration = (seconds) => {
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return h ? tf('{} h {} min', h, m) : tf('{} min', m);
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
      li.append(text, el('div', 'amount', tf('{} · {} plays', duration(item.seconds), item.plays)));
      return li;
    }),
  );
  if (!items.length) list.append(el('li', 'empty', t('Nothing yet.')));
};
// A point of the series: a day (days since 1970, local) or a month.
const pointLabel = (p) => (p.day !== undefined ? String(new Date(p.day * 86400000).getUTCDate()) : month(p.month));
const pointTitle = (p) =>
  p.day !== undefined
    ? new Date(p.day * 86400000).toLocaleDateString(undefined, { timeZone: 'UTC' })
    : `${month(p.month)} ${p.year}`;
const renderStats = (s) => {
  $('stats-tiles').replaceChildren(
    tile(duration(s.seconds), t('listened')),
    tile(String(s.plays), tf('plays ({} skipped)', s.skips)),
    tile(String(s.artists), s.new_artists ? tf('artists, {} new', s.new_artists) : t('artists')),
    tile(tf('{} d', s.streak), tf('streak (best {} d)', s.longest_streak)),
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
    heat.append(el('span', 'day', t(DAYS[d])));
    hours.forEach((seconds, h) => {
      const cell = el('i');
      cell.style.opacity = String(0.08 + 0.92 * (seconds / hottest));
      cell.title = `${t(DAYS[d])} ${h}:00: ${duration(seconds)}`;
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
      const o = el('option', '', PRESET_NAMES[id] ? t(PRESET_NAMES[id]) : id);
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
const transitionFields = (box, transition, max, onChange) => {
  const length = el('input');
  length.type = 'range';
  length.min = '0';
  length.max = String(max);
  length.step = '500';
  length.value = String(transition.duration_ms);
  const shown = el('span', 'value', seconds(transition.duration_ms));
  const lengthLabel = el('label', '', `${t('Length')} `);
  lengthLabel.append(shown, length);
  const curve = el('select');
  for (const [id, name] of Object.entries(CURVE_NAMES)) {
    const o = el('option', '', t(name));
    o.value = id;
    curve.append(o);
  }
  curve.value = transition.curve;
  const curveLabel = el('label', '', `${t('Curve')} `);
  curveLabel.append(curve);
  const effects = Object.entries(EFFECT_NAMES).map(([id, english]) => {
    const name = t(english);
    const row = el('div', 'option');
    const body = el('div', 'body');
    body.append(el('div', 'name small', name));
    const toggle = el('input', 'switch');
    toggle.type = 'checkbox';
    toggle.checked = !!transition.effects?.[id];
    toggle.setAttribute('aria-label', name);
    toggle.dataset.effect = id;
    row.append(body, toggle);
    return row;
  });
  const read = () => ({
    ...transition,
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
      const o = el('option', '', l === m.current ? tf('{} (playing)', l) : l);
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
      const button = el('button', 'secondary', t(edited.has(`${a.id}>${b.id}`) ? 'Edited' : 'Auto'));
      button.type = 'button';
      button.addEventListener('click', () => editPair(a, b));
      li.append(text, button);
      return li;
    }),
  );
  if (items.length < 2) $('mix-pairs').append(el('li', 'empty', t('Play this playlist once to list its tracks.')));
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
    tf('Tempo {}', tempo),
    match === null
      ? t('Keys not measured yet')
      : tf(match ? 'Keys {} → {}: they mix well' : 'Keys {} → {}: they may clash', editFacts.a.key, editFacts.b.key),
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
  g.fillText(out !== null ? tf('out {}', time(out)) : t('out: end of the track'), 6, 12);
  g.fillText(tf('in {}', time(editing.in_ms)), 6, lane + 12);
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
  if (!confirm(t("Replace this playlist's mix with the pasted one?"))) return;
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
      if (isNew) name.append(el('span', 'badge', t('new')));
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

// Remote (remote.rs), through settings_remote. The section asks for its state when the window loads and
// whenever Rust says it changed (the `remote-changed` event, sent by settings_window::remote_changed): no
// polling. The pairing countdown is the only timer; it runs while a pairing window is open, the section is
// on screen and the window is visible, and counts from the deadline, so it resumes where it was. The
// pairing is closed when the section leaves the screen (and by Rust when the window closes).
// Names and codes come from a phone: shown with textContent only.
const remoteCall = async (op, arg = {}) => {
  try {
    const r = await invoke('settings_remote', { op, arg });
    if (op !== 'get') showError('');
    return r ?? true;
  } catch (e) {
    showError(String(e));
    return null;
  }
};
const loadRemote = async () => {
  const r = await remoteCall('get');
  if (r && r !== true) renderRemote(r);
};

const clock = (s) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
const setText = (id, text) => {
  if ($(id).textContent !== text) $(id).textContent = text;
};

// The QR from Rust's modules (`size`² characters of 0 and 1): one path of runs, with a quiet zone of 4.
// Each module is a whole number of pixels, so its edges stay sharp.
let drawnQr = null;
const drawQr = ({ size, modules }) => {
  let d = '';
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      if (modules[y * size + x] !== '1') continue;
      let n = 1;
      while (x + n < size && modules[y * size + x + n] === '1') n++;
      d += `M${x} ${y}h${n}v1h-${n}z`;
      x += n;
    }
  }
  const NS = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', `-4 -4 ${size + 8} ${size + 8}`);
  svg.setAttribute('shape-rendering', 'crispEdges');
  svg.setAttribute('role', 'img');
  svg.setAttribute('aria-label', t('QR code for pairing a phone'));
  const ground = document.createElementNS(NS, 'rect');
  for (const [name, value] of Object.entries({ x: -4, y: -4, width: size + 8, height: size + 8 })) {
    ground.setAttribute(name, String(value));
  }
  const dark = document.createElementNS(NS, 'path');
  dark.setAttribute('d', d);
  svg.append(ground, dark);
  const side = Math.max(3, Math.floor(264 / (size + 8))) * (size + 8);
  $('pair-qr').style.setProperty('width', `${side}px`);
  $('pair-qr').style.setProperty('height', `${side}px`);
  $('pair-qr').replaceChildren(svg);
};

// Function declarations: showTab runs at load (a `#tab` in the URL), before the rest of this block.
function syncTimer() {
  const run = pairUntil > 0 && $('remote-box').offsetParent !== null && document.visibilityState === 'visible';
  if (run && !pairTimer) {
    pairTimer = setInterval(tickPairing, 1000);
    tickPairing();
  } else if (!run && pairTimer) {
    clearInterval(pairTimer);
    pairTimer = 0;
  }
}
function tickPairing() {
  const left = Math.max(0, Math.ceil((pairUntil - Date.now()) / 1000));
  setText('pair-left', tf('Expires in {}', clock(left)));
  if (left === 0) stopPairing();
}
async function stopPairing() {
  qr = null;
  pairUntil = 0;
  syncTimer();
  await remoteCall('pair_stop');
  loadRemote();
}
// Called when the tab or the search changes what is on screen.
function syncPairing() {
  if (remote?.pairing && $('remote-box').offsetParent === null) stopPairing();
  else syncTimer();
}
document.addEventListener('visibilitychange', syncTimer);

const day = (s) => new Date(s * 1000).toLocaleDateString(undefined, { dateStyle: 'medium' });
const moment = (s) => new Date(s * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const deviceRow = (d) => {
  const li = el('li');
  const body = el('div', 'body');
  const seen = d.connected ? t('Connected now') : d.last_seen ? tf('Last seen {}', moment(d.last_seen)) : t('Not seen since the app started');
  body.append(el('div', 'title', d.name), el('div', 'detail', [tf('Paired {}', day(d.paired)), seen].join(' · ')));
  const remove = el('button', 'secondary', t('Remove'));
  remove.type = 'button';
  remove.setAttribute('aria-label', tf('Remove {}', d.name));
  remove.addEventListener('click', async () => {
    await remoteCall('remove', { key: d.key });
    loadRemote();
  });
  li.append(el('span', d.connected ? 'dot on' : 'dot'), body, remove);
  return li;
};

const renderRemote = (r) => {
  remote = r;
  const pairing = r.pairing;
  // A QR is good until a phone scans it; the pairing window ends with its answer.
  if (!pairing || !pairing.fresh) qr = null;
  pairUntil = pairing ? Date.now() + pairing.left * 1000 : 0;
  const stage = r.pending ? 'ask' : pairing && (qr || !pairing.fresh) ? 'open' : 'idle';
  $('remote-off').hidden = r.listening;
  $('pair-start').disabled = !r.listening;
  $('pair-idle').hidden = stage !== 'idle';
  $('pair-open').hidden = stage !== 'open';
  $('pair-ask').hidden = stage !== 'ask';
  if (stage === 'open') {
    $('pair-qr').hidden = !qr;
    if (qr && drawnQr !== qr) drawQr((drawnQr = qr));
    setText('pair-title', t(qr ? 'Scan with the feathertune app' : 'Phone connected'));
    setText(
      'pair-hint',
      t(qr ? 'Open feathertune on your phone and point its camera at this code. The phone and this PC must be on the same network.' : 'Finishing the secure connection…'),
    );
  } else {
    drawnQr = null;
  }
  if (stage === 'ask') {
    setText('pair-ask-title', tf('Allow {} to control feathertune?', r.pending.name));
    setText('pair-code', `${r.pending.code.slice(0, 3)} ${r.pending.code.slice(3)}`);
    $('pair-code').setAttribute('aria-label', tf('Code {}', r.pending.code.split('').join(' ')));
  }
  $('remote-devices').replaceChildren(...(r.devices.length ? r.devices.map(deviceRow) : [el('li', 'empty', t('No phones paired yet.'))]));
  $('remote-forget').hidden = !r.devices.length;
  syncTimer();
};

$('pair-start').addEventListener('click', async () => {
  $('pair-start').disabled = true;
  const r = await remoteCall('pair_start');
  if (r && r !== true) qr = { size: r.size, modules: r.modules };
  await loadRemote();
});
$('pair-cancel').addEventListener('click', stopPairing);
const answerPairing = async (allow) => {
  const code = remote?.pending?.code;
  if (!code) return;
  await remoteCall('answer', { allow, code });
  loadRemote();
};
$('pair-allow').addEventListener('click', () => answerPairing(true));
$('pair-deny').addEventListener('click', () => answerPairing(false));
$('remote-forget').addEventListener('click', async () => {
  if (!confirm(t('Forget all paired phones? Each one has to be paired again.'))) return;
  await remoteCall('forget_all');
  loadRemote();
});
window.addEventListener('remote-changed', loadRemote);

const load = async () => {
  try {
    const s = await invoke('settings_get');
    if (!document.documentElement.dataset.lang) {
      strings = s.strings || {};
      document.documentElement.lang = document.documentElement.dataset.lang = s.ui.active;
      translatePage();
      for (const l of s.ui.languages) {
        const o = el('option', '', l.name);
        o.value = l.tag;
        $('language').append(o);
      }
    }
    applyTheme(s.theme);
    renderChangelog(s.changelog, s.updated_from);
    renderPlugins(s.plugins);
    loadRemote();
    renderLastFm(s.lastfm);
    $('discord-id').textContent = t(
      s.discord.built_in
        ? 'Optional: this build has its own; an id typed here takes its place.'
        : 'Rich Presence is off while this is empty.',
    );
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
      if ('shortcut' in input.dataset) input.placeholder = t('Off');
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
    input.value = t('Press keys…');
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
      input.value = t('Add Ctrl, Alt, Shift or Win');
      return;
    }
    const keys = [...mods, keyName(e.code)].join('+');
    stop(keys);
    if (await set(input.dataset.key, keys, input)) flashSaved(input);
    else input.value = before;
  });
}

// Extensions (extensions.rs): the list, each one's switch and settings, and the import (a folder, a .zip read
// here, or one file), all through settings_extensions. Their texts are shown with textContent only.
const extCall = (op, arg) => invoke('settings_extensions', { op, arg: arg ?? null });
const PERMISSIONS = {
  'player.control': 'Control the player (play, pause, skip, seek, volume)',
  storage: 'Keep its own data on this PC',
  notify: 'Show notifications',
};
const permissionText = (p) => (p.startsWith('net:') ? tf('Send requests to {}', p.slice(4)) : t(PERMISSIONS[p] || p));
const asks = (x) => x.permissions.filter((p) => p !== 'player.read');
const confirmEnable = (x) =>
  confirm(
    [
      tf('Turn on {}?', x.name),
      '',
      t(
        'It runs inside the player page with your signed-in account and can do anything you can do there. feathertune does not review extensions: only turn on what you trust.',
      ),
      ...(asks(x).length ? ['', t('It also asks to:'), ...asks(x).map((p) => `• ${permissionText(p)}`)] : []),
      ...(x.wasm ? ['', t('It has native code (WebAssembly), run in a sandbox.')] : []),
    ].join('\n'),
  );
const extStatus = (message) => {
  $('ext-status').textContent = message || '';
  $('ext-status').hidden = !message;
};

const extSettings = (x) => {
  const box = el('div', 'ext-settings');
  const values = { ...x.values };
  const save = async (input) => {
    try {
      await extCall('set', { id: x.id, values });
      input.classList.remove('invalid');
      showError('');
      flashSaved(input);
    } catch (e) {
      input.classList.add('invalid');
      showError(String(e));
    }
  };
  for (const d of x.settings) {
    if (d.type === 'bool') {
      const input = el('input', 'switch');
      input.type = 'checkbox';
      input.checked = !!values[d.key];
      input.setAttribute('aria-label', d.label);
      input.addEventListener('change', () => {
        values[d.key] = input.checked;
        save(input);
      });
      const body = el('div', 'body');
      body.append(el('div', 'name small', d.label));
      if (d.hint) body.append(el('p', 'hint', d.hint));
      const row = el('div', 'option');
      row.append(body, input);
      box.append(row);
      continue;
    }
    let input;
    if (d.type === 'enum') {
      input = el('select');
      for (const o of d.options) {
        const option = el('option', '', o);
        option.value = o;
        input.append(option);
      }
    } else {
      input = el('input');
      input.type = { number: 'number', color: 'color' }[d.type] || 'text';
      if (d.type === 'number') {
        if (d.min != null) input.min = d.min;
        if (d.max != null) input.max = d.max;
        if (d.step != null) input.step = d.step;
      } else if (d.type === 'string') {
        input.maxLength = d.max ?? 200;
        input.spellcheck = false;
      }
    }
    input.value = values[d.key];
    input.addEventListener('change', () => {
      values[d.key] = d.type === 'number' ? Number(input.value) : input.value;
      save(input);
    });
    const label = el('label', '', d.label);
    label.append(input);
    box.append(label);
    if (d.hint) box.append(el('p', 'hint', d.hint));
  }
  return box;
};

const extCard = (x) => {
  const card = el('div', x.on ? 'card ext' : 'card ext off');
  const body = el('div', 'body');
  const name = el('div', 'name', x.name);
  name.append(el('span', 'badge', x.version));
  if (x.on !== x.running) name.append(el('span', 'badge', t('restart')));
  body.append(name, el('div', 'hint', [x.author && tf('by {}', x.author), x.id].filter(Boolean).join(' · ')));
  if (x.description) body.append(el('div', 'description', x.description));
  const toggle = el('input', 'switch');
  toggle.type = 'checkbox';
  toggle.checked = x.on;
  toggle.setAttribute('aria-label', x.name);
  toggle.addEventListener('change', async () => {
    if (toggle.checked && !confirmEnable(x)) {
      toggle.checked = false;
      return;
    }
    try {
      await extCall('toggle', { id: x.id, on: toggle.checked });
      showError('');
      loadExtensions();
      $('restart').hidden = false;
    } catch (e) {
      toggle.checked = !toggle.checked;
      showError(String(e));
    }
  });
  const row = el('div', 'row');
  if (x.icon) {
    const icon = el('img', 'ext-icon');
    icon.src = x.icon;
    icon.alt = '';
    row.append(icon);
  }
  row.append(body, toggle);

  const extra = el('div', 'extra');
  const kinds = [x.page && t('page code'), x.css && t('styles'), x.wasm && t('native code (WebAssembly)')].filter(Boolean);
  extra.append(el('p', 'hint', tf('Runs: {}', kinds.join(', '))));
  if (asks(x).length) {
    const list = el('ul', 'ext-permissions');
    list.append(...asks(x).map((p) => el('li', '', permissionText(p))));
    extra.append(list);
  }
  if (x.settings.length) extra.append(extSettings(x));
  if (x.errors.length) {
    const errors = el('details', 'ext-errors');
    errors.append(el('summary', '', tf('{} errors', x.errors.length)), ...x.errors.map((m) => el('pre', '', m)));
    extra.append(errors);
  }
  const remove = el('button', 'secondary', t('Remove'));
  remove.type = 'button';
  remove.addEventListener('click', async () => {
    if (!confirm(tf('Remove {} and the data it keeps? This cannot be undone.', x.name))) return;
    try {
      await extCall('remove', { id: x.id });
      showError('');
      if (x.running) $('restart').hidden = false;
      loadExtensions();
    } catch (e) {
      showError(String(e));
    }
  });
  const actions = el('div', 'actions');
  actions.append(remove);
  extra.append(actions);
  card.append(row, extra);
  return card;
};

let extensions = null; // the last settings_extensions `list`
const loadExtensions = async () => {
  try {
    extensions = await extCall('list');
  } catch (e) {
    showError(String(e));
    return;
  }
  $('ext-list').replaceChildren(...extensions.list.map(extCard));
  $('ext-broken').replaceChildren(
    ...extensions.broken.map((b) => {
      const card = el('div', 'card off');
      card.append(el('div', 'name', b.name), el('p', 'hint', tf('Not loaded: {}', b.error)));
      return card;
    }),
  );
};

// A .zip's files as [[path, ArrayBuffer]]: stored or deflate entries only (DecompressionStream), at most 32 MB.
const MAX_PACKAGE = 32 * 1024 * 1024;
const unzip = async (buffer) => {
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  let end = -1;
  for (let i = buffer.byteLength - 22; i >= Math.max(0, buffer.byteLength - 65557); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      end = i;
      break;
    }
  }
  if (end < 0) throw new Error(t('This is not a zip file.'));
  const count = view.getUint16(end + 10, true);
  let p = view.getUint32(end + 16, true);
  const decoder = new TextDecoder();
  const out = [];
  let total = 0;
  for (let n = 0; n < count; n++) {
    if (p + 46 > buffer.byteLength || view.getUint32(p, true) !== 0x02014b50) throw new Error(t('The zip file is damaged.'));
    const flags = view.getUint16(p + 8, true);
    const method = view.getUint16(p + 10, true);
    const size = view.getUint32(p + 20, true);
    const full = view.getUint32(p + 24, true);
    const nameLength = view.getUint16(p + 28, true);
    const local = view.getUint32(p + 42, true);
    const name = decoder.decode(bytes.subarray(p + 46, p + 46 + nameLength));
    p += 46 + nameLength + view.getUint16(p + 30, true) + view.getUint16(p + 32, true);
    if (name.endsWith('/') || name.startsWith('__MACOSX/') || name.endsWith('.DS_Store')) continue;
    if (flags & 1) throw new Error(t('Encrypted zip files are not supported.'));
    total += full;
    if (size === 0xffffffff || total > MAX_PACKAGE) throw new Error(t('The package is over 32 MB.'));
    const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const data = bytes.subarray(start, start + size);
    if (method === 0) out.push([name, data.slice().buffer]);
    else if (method === 8) {
      const stream = new Blob([data]).stream().pipeThrough(new DecompressionStream('deflate-raw'));
      out.push([name, await new Response(stream).arrayBuffer()]);
    } else throw new Error(t('This zip file uses a compression that is not supported.'));
  }
  return out;
};
const base64 = (buffer) => {
  const bytes = new Uint8Array(buffer);
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
const importFiles = async (entries) => {
  extStatus('');
  try {
    const files = {};
    for (const [path, buffer] of entries) files[path] = base64(buffer);
    const r = await extCall('import', { files });
    showError('');
    await loadExtensions();
    const x = extensions?.list.find((e) => e.id === r.id);
    if (r.previous) {
      extStatus(tf('{} updated from {} to {}.', r.name, r.previous, r.version));
      if (x?.on) $('restart').hidden = false;
    } else {
      extStatus(tf('{} {} imported.', r.name, r.version));
      if (x && confirmEnable(x)) {
        await extCall('toggle', { id: x.id, on: true });
        $('restart').hidden = false;
        await loadExtensions();
      }
    }
  } catch (e) {
    showError(String(e));
  }
};
const importPicked = async (file) => {
  const buffer = await file.arrayBuffer();
  if (!/\.zip$/i.test(file.name)) return importFiles([[file.name, buffer]]);
  try {
    importFiles(await unzip(buffer));
  } catch (e) {
    showError(String(e.message || e));
  }
};
$('ext-pick-file').addEventListener('click', () => $('ext-file').click());
$('ext-pick-folder').addEventListener('click', () => $('ext-folder').click());
$('ext-open-folder').addEventListener('click', () => extCall('open_folder').catch((e) => showError(String(e))));
$('ext-file').addEventListener('change', async (e) => {
  const file = e.target.files[0];
  e.target.value = '';
  if (file) importPicked(file);
});
$('ext-folder').addEventListener('change', async (e) => {
  const files = [...e.target.files];
  e.target.value = '';
  if (files.length) importFiles(await Promise.all(files.map(async (f) => [f.webkitRelativePath || f.name, await f.arrayBuffer()])));
});
// A dropped folder is walked here (FileSystemEntry); a dropped file is read like a picked one.
const readEntry = (entry, prefix, out) =>
  new Promise((resolve, reject) => {
    if (entry.isFile) {
      entry.file(async (f) => {
        out.push([prefix + f.name, await f.arrayBuffer()]);
        resolve();
      }, reject);
      return;
    }
    const reader = entry.createReader();
    const children = [];
    const next = () =>
      reader.readEntries(async (batch) => {
        if (batch.length) {
          children.push(...batch);
          next();
          return;
        }
        for (const child of children) await readEntry(child, `${prefix}${entry.name}/`, out);
        resolve();
      }, reject);
    next();
  });
const panel = document.querySelector('[data-panel="extensions"]');
panel.addEventListener('dragover', (e) => {
  e.preventDefault();
  panel.classList.add('dropping');
});
panel.addEventListener('dragleave', () => panel.classList.remove('dropping'));
panel.addEventListener('drop', async (e) => {
  e.preventDefault();
  panel.classList.remove('dropping');
  const entry = e.dataTransfer.items[0]?.webkitGetAsEntry?.();
  if (!entry) return;
  if (entry.isFile) {
    entry.file(importPicked, (err) => showError(String(err)));
    return;
  }
  const out = [];
  try {
    await readEntry(entry, '', out);
  } catch (err) {
    showError(String(err));
    return;
  }
  importFiles(out);
});

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
$('diagnostics-export').addEventListener('click', () => action('diagnostics_export'));
$('stats-clear').addEventListener('click', async () => {
  if (!confirm(t('Delete the whole listening history? This cannot be undone.'))) return;
  await action('stats_clear');
  loadStats();
});
$('restart-button').addEventListener('click', () => action('restart'));

// The Last.fm sign-in finishes in the browser, and the track (its colors) may change: refresh when
// the window comes back.
window.addEventListener('focus', load);
load();
