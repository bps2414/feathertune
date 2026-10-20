# feathertune

*Português: [README.pt-BR.md](README.pt-BR.md)*

> **Work in progress, experimental.** This is a personal project that I am still testing heavily. Things break, change or get rewritten without notice. There are no releases and no support yet; CI builds an installer; only its silent install and uninstall were tested. Use it at your own risk.

A lightweight Windows desktop shell for a web music player, written in Rust with Tauri 2. Instead of bundling Chromium (like Electron apps do) it uses the WebView2 runtime that already ships with Windows, and handles the desktop parts natively: its own title bar, tray icon, media keys and the Windows media flyout, an always-on-top mini player, and memory trimming while hidden.

Measured so far (process tree, playing): ~410 MB private memory and ~3% of one CPU core with the window open, ~170 MB of RAM while in the tray, about 2 s to start. Details in [docs/PLAN.md](docs/PLAN.md).

## What it does and does not do

- Loads the service's **official website**, unmodified. You sign in on the site itself; the app never sees or stores your credentials.
- Reads the current track from the page only to show it in the media flyout and tray.
- Does **not** block or alter ads, download or extract audio, use private APIs, or bypass any protection. Some features may require a subscription to the service, exactly as in a browser.

## Status

| Phase | State |
|---|---|
| 1. Shell: window, tray, memory trimming | done |
| 2. Player state bridge, media keys, flyout | done |
| 3. Integrations (single instance, autostart, shortcuts, Discord, Last.fm) | tray and shortcuts checked; Discord and Last.fm still to be confirmed on a real profile |
| 4. Slimming and installer | measured (~400 MB private, ~3.3% CPU, ~190 MB in the tray); silent install/uninstall checked; Electron comparison and the "delete data" uninstall path open |
| 5. Native shell: own title bar, settings window and plugins, mini player | built and measured (~430 MB private, ~4.5% CPU open, above Phase 4 pending a same-day baseline); snap/150%/second monitor and Last.fm sign-in on a CI build to be confirmed by the maintainer |
| 6. Look: album color, synced lyrics, ambient mode | planned |

## Build

Windows 10/11 with the WebView2 runtime and a Rust toolchain (stable, MSVC).

```powershell
cd app
cargo build --release
.\target\release\feathertune.exe
```

`cargo check` is the fast loop while editing; `cargo build --profile dist` is the slow, fully optimized build meant for releases.

Installer (NSIS, per user, no admin): `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`, or download it from the CI run. Uninstalling asks whether to also delete your data (sign-in, `config.toml`); it is kept unless you tick the box.

## Configuration

`%LOCALAPPDATA%\feathertune\config.toml` (created on first run with the plugin list, outside the repository). The settings window (title bar or tray menu > Settings) edits everything below and keeps your comments; editing by hand works too. Everything is optional:

```toml
[plugins]                    # every optional feature; the settings window shows its description and cost
controls = true              # page module that presses the player's buttons
tracker_filter = true        # blocks third-party analytics requests; never ads (docs/PLAN.md)
track_notifications = true   # toast on track change, only while the window is hidden
global_shortcuts = true
discord = true               # also needs client_id below
lastfm = true                # also needs your account connected
mini_player = true
animated_title_bar = true     # slow color drift; pauses when hidden, minimized or in the background

[shortcuts]                  # global; "" turns one off
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # opens/closes the mini player

[discord]
client_id = ""               # your own Discord application id

[lastfm]
api_key = ""                 # optional: CI builds carry their own key; these values take priority
api_secret = ""

[browser]
disable_features = []        # extra Chromium features to turn off (experiments, see docs/PLAN.md)
```

The old keys `[injections] controls`, `[privacy] tracker_filter` and `[notifications] track_change` are still read; the settings window replaces them with the `[plugins]` entry. "Start with Windows", "Connect Last.fm" and the mini player are also in the tray menu. Last.fm sign-in happens in your browser; the session key is kept in the Windows Credential Manager, never in a file. A local build without a Last.fm key needs `api_key` and `api_secret` here.

## Disclaimer

This is an independent, unofficial project. It is not affiliated with, endorsed by or sponsored by any music streaming service or its owners. All trademarks belong to their respective owners. You are responsible for using it in line with the terms of the service you sign in to.

No license has been chosen yet, so all rights are reserved for now.
