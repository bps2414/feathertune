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
const showTab = (name) => {
  for (const t of tabs) t.setAttribute('aria-selected', String(t.dataset.tab === name));
  for (const p of document.querySelectorAll('[role="tabpanel"]')) p.hidden = p.dataset.panel !== name;
};
for (const t of tabs) t.addEventListener('click', () => showTab(t.dataset.tab));

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
    body.append(name, el('div', 'description', p.description), el('div', 'cost', `Cost: ${p.cost}`));
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

const load = async () => {
  try {
    const s = await invoke('settings_get');
    applyTheme(s.theme);
    renderPlugins(s.plugins);
    renderLastFm(s.lastfm);
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
$('restart-button').addEventListener('click', () => action('restart'));

// The Last.fm sign-in finishes in the browser, and the track (its colors) may change: refresh when
// the window comes back.
window.addEventListener('focus', load);
load();
