# Writing a feathertune extension

An extension is a folder you import in Settings > Extensions. It can restyle the player page (CSS), run code in it (JavaScript) and run native code in a sandbox (WebAssembly). pt-BR: [EXTENSIONS.pt-BR.md](EXTENSIONS.pt-BR.md). How the app runs them: [DESIGN.md](DESIGN.md#extensions). Examples: [`examples/extensions/`](../examples/extensions/).

Extensions are your own code: the app does not review them. Import only what you have read or trust.

## Package

```
my-extension/
  plugin.json   the manifest (required)
  style.css     added to the player page
  page.js       runs in the player page
  plugin.wasm   runs in a WebAssembly sandbox
  icon.png      at most 64 KB
```

At least one of `style.css`, `page.js` and `plugin.wasm`. Import the folder, a `.zip` of it (stored or deflate) or, for the simplest cases, one file:

- a `.css` alone becomes a theme (id `local.<file name>`);
- a `.js` alone takes its manifest from a header at the top:

```js
// ==Feathertune==
// @id yourname.my-script
// @name My script
// @version 1.0.0
// @permission storage
// ==/Feathertune==
```

Limits: page code 2 MB, WebAssembly 16 MB, the package 32 MB, 100 extensions.

An imported extension is off until you switch it on (the dialog lists what it asks for). Importing, switching and removing apply after the app restarts; settings apply at once. Importing a new version of the same id keeps its switch, settings and data. Errors show under the extension in the tab and in the diagnostics.

## Manifest

```json
{
  "id": "yourname.my-extension",
  "name": "My extension",
  "version": "1.0.0",
  "author": "Your name",
  "description": "What it does, in one or two sentences.",
  "api": 1,
  "permissions": ["storage", "net:api.example.com"],
  "settings": [
    { "key": "size", "type": "number", "label": "Size", "default": 12, "min": 8, "max": 32, "step": 1 }
  ],
  "i18n": { "pt-BR": { "My extension": "Minha extensão", "Size": "Tamanho" } }
}
```

- `id`: two or more parts separated by dots, each of `a-z`, `0-9`, `_` and `-`; 3 to 64 characters. The folder it is imported into takes this name.
- `name` (1 to 64 characters), `version` (like `1.2.0`), `author`, `description` (up to 500).
- `api`: the version of the extension API it needs (now `1`); an app with an older one refuses it.
- `i18n`: language tag (`pt-BR`) → English text → translation, for the name, description and setting labels and hints.

### Permissions

| Permission | Lets it |
|---|---|
| `player.read` | Nothing more: every extension reads the player state. Write it to say that it does. |
| `player.control` | Play, pause, skip, seek and set the volume (refused during an ad, like any caller). |
| `storage` | Keep up to 1 MB of JSON values, its own only. |
| `notify` | Show Windows notifications, one per 10 s. |
| `net:api.example.com` | Make https requests to that host. `net:*.example.com` is the host and its subdomains. |

### Settings

Up to 32, drawn in the Extensions tab. Each has `key` (1 to 32 of `a-z`, `0-9`, `_`), `type`, `label`, `hint` and `default`, plus:

| `type` | Value | Extra fields |
|---|---|---|
| `bool` | `true`/`false` | |
| `number` | a number | `min`, `max`, `step` |
| `enum` | one of `options` | `options` (1 to 50 texts) |
| `string` | text without control characters | `max` (length, default 200) |
| `color` | `#rrggbb` | |

## Page code

`style.css` is added to the page as it is. `page.js` runs after the app's own modules, once per page load, in the page's own JavaScript world, wrapped in a function that gets `ft`, the extension's API. Typed in [`sdk/feathertune.d.ts`](../sdk/feathertune.d.ts).

```js
ft.on('track', (state) => ft.log('now playing', state.title, 'by', state.artist));
```

- `ft.id`, `ft.version`, `ft.api`, `ft.permissions`, `ft.settings` (the values of now), `ft.onSettings(fn)`.
- `ft.on(event, fn)` returns a function that removes `fn`. Events:
  - `state`: any change of the player state; `track`: a new track; `play`, `pause`, `seek`; `ad`: `true` when an ad starts, `false` when it ends;
  - `queue`: `{queue, current, next, list}`;
  - `lyrics`, `colors`: the synced lyrics and the album colors, when the app's own Synced lyrics and Album color plugins are on;
  - `settings`: the new values; `native`: a message from its WebAssembly.
- `ft.player.state()`: `{id, title, artist, album, artwork, duration, position, paused, volume, muted, ad, list}` or `null`. `ft.player.queue()`, `ft.player.video()` (the `<video>`), `ft.player.clock` (`time(video)`, the exact position).
- With `player.control`: `ft.player.play()`, `pause()`, `playPause()`, `next()`, `previous()`, `seek(seconds)`, `setVolume(0..100)`, `volumeUp()`, `volumeDown()`; each returns `false` when refused.
- `ft.theme.colors()`, `ft.lyrics.current()`.
- `ft.addStyle(css)` returns the `<style>`. `ft.log(...)` goes to the console and the diagnostics (30 lines a minute); `ft.error(e)` shows `e` in the Extensions tab.
- `ft.storage.get(key)`, `set(key, value)`, `remove(key)`, `keys()`: promises (`storage`).
- `ft.fetch(url, {method, headers, body})` → a promise of `{status, headers, body}` (body as text). https only, to its `net:` hosts, made by the app (so CORS does not apply): no redirects, no cookies, 10 s, 1 MB out, 2 MB back, 60 a minute.
- `ft.notify(title, body)`: a promise (`notify`).
- `ft.native.available`, `ft.native.send(data)`: a message to its WebAssembly (256 KB).

Unhandled errors in its callbacks are caught and shown in the tab; one that never started (a syntax error) is reported as not started.

## WebAssembly

`plugin.wasm` runs in an interpreter on a thread of its own, with no WASI: it reaches nothing but the functions below. Rust: [`sdk/rust`](../sdk/rust) (the `feathertune-ext` crate) wraps all of this; see [`examples/extensions/track-counter`](../examples/extensions/track-counter).

```
cargo build --release --target wasm32-unknown-unknown
```

then copy `target/wasm32-unknown-unknown/release/<name>.wasm` to `plugin.wasm` next to `plugin.json`.

Any language works if the module has this ABI (all JSON in UTF-8):

- Exports: `memory`, `ft_alloc(len: i32) -> i32` (memory for the app to write an event into) and `ft_event(ptr: i32, len: i32)` (one event; the module frees it).
- Imports (module `ft`):
  - `call(ptr: i32, len: i32) -> i32`: a request `{"op": ..., ...}`; returns the length of the answer, `{"ok": value}` or `{"error": "text"}`;
  - `take(ptr: i32, cap: i32) -> i32`: copies that answer into memory and returns the bytes copied;
  - `log(ptr: i32, len: i32)`: a line in the diagnostics.
- Events (`type`): `init` (`id`, `version`, `settings`, `app_version`), `track` and `state` (`state`), `ad` (`ad`, `state`), `settings` (`settings`), `page` (`data`, from its page code), `timer` (`id`).
- Requests (`op`): `state`, `queue`, `settings`, `now` (ms since 1970), `command` (`name`: play, pause, playPause, next, previous, seek, volume; `value` for the last two; `player.control`), `timer` (`ms` from 10 to 86 400 000, `id`; at most 16 waiting), `page_send` (`data`, to `ft.on('native')`), `storage_get`/`storage_set`/`storage_remove`/`storage_keys` (`key`, `value`), `fetch` (`url`, `method`, `headers`, `body`), `notify` (`title`, `body`), `log` and `error` (`message`).

Limits: 64 MB of memory; the start and each event may run for about 2·10⁹ instructions. A trap, running past that or a bad pointer stops the module until the app restarts, and the error shows in the tab.
