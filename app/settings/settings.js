// The settings window. Talks to Rust only through the settings_* commands, which only this window
// may call (capabilities/settings-window.json). Every change is saved as soon as it is made.
'use strict';

const invoke = (cmd, args) => window.__TAURI_INTERNALS__.invoke(cmd, args);
const $ = (id) => document.getElementById(id);

const showError = (message) => {
  const el = $('error');
  el.textContent = message || '';
  el.hidden = !message;
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

const renderPlugins = (plugins) => {
  const list = $('plugins');
  list.replaceChildren(
    ...plugins.map((p) => {
      const li = document.createElement('li');
      const toggle = document.createElement('input');
      toggle.type = 'checkbox';
      toggle.className = 'switch';
      toggle.checked = p.on;
      toggle.setAttribute('aria-label', p.name);
      toggle.addEventListener('change', async () => {
        if (!(await set(`plugins.${p.id}`, toggle.checked, null))) toggle.checked = !toggle.checked;
      });
      const body = document.createElement('div');
      body.className = 'body';
      const name = document.createElement('div');
      name.className = 'name';
      name.textContent = p.name;
      if (!p.live) {
        const badge = document.createElement('span');
        badge.className = 'badge';
        badge.textContent = 'restart';
        name.append(badge);
      }
      const description = document.createElement('div');
      description.className = 'description';
      description.textContent = p.description;
      const cost = document.createElement('div');
      cost.className = 'cost';
      cost.textContent = `Cost: ${p.cost}`;
      body.append(name, description, cost);
      li.append(toggle, body);
      return li;
    }),
  );
};

const renderLastFm = (l) => {
  const status = $('lastfm-status');
  const button = $('lastfm-button');
  button.hidden = l.connected === null;
  if (l.connected === null) {
    status.textContent = 'Not running (plugin off or no API keys when the app started).';
  } else {
    status.textContent = l.connected ? 'Connected' : 'Not connected';
    button.textContent = l.connected ? 'Disconnect' : 'Connect';
    button.dataset.action = l.connected ? 'lastfm_disconnect' : 'lastfm_connect';
  }
  $('lastfm-keys').textContent = l.built_in
    ? 'This build has its own API keys; keys typed below take their place.'
    : 'This build has no API keys of its own: add yours below (from last.fm/api).';
};

const load = async () => {
  try {
    const s = await invoke('settings_get');
    renderPlugins(s.plugins);
    renderLastFm(s.lastfm);
    $('restart').hidden = !s.restart_needed;
    for (const input of document.querySelectorAll('[data-key]')) {
      if (input === document.activeElement) continue;
      const value = input.dataset.key.split('.').reduce((o, k) => o?.[k], s);
      if (input.type === 'checkbox') input.checked = !!value;
      else input.value = Array.isArray(value) ? value.join(', ') : (value ?? '');
    }
  } catch (e) {
    showError(String(e));
  }
};

for (const input of document.querySelectorAll('[data-key]')) {
  input.addEventListener('change', () => {
    let value = input.type === 'checkbox' ? input.checked : input.value;
    if ('list' in input.dataset) value = value.split(',').map((x) => x.trim()).filter(Boolean);
    set(input.dataset.key, value, input);
  });
}

$('lastfm-button').addEventListener('click', async (e) => {
  try {
    await invoke('settings_action', { action: e.currentTarget.dataset.action });
    showError('');
  } catch (err) {
    showError(String(err));
  }
  setTimeout(load, 500);
});

$('mini-player').addEventListener('click', () => invoke('settings_action', { action: 'mini_player' }).catch((e) => showError(String(e))));

// The Last.fm sign-in finishes in the browser: refresh when the window comes back.
window.addEventListener('focus', load);
load();
