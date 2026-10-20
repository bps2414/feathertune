# feathertune

*Português: [README.pt-BR.md](README.pt-BR.md)*

> **Stable for daily use (0.1.x).** A personal project, used every day and updated through GitHub releases; the app installs new versions on its own. There is no formal support.

A lightweight Windows desktop shell for a web music player, written in Rust with Tauri 2. Instead of bundling Chromium (like Electron apps do) it uses the WebView2 runtime that already ships with Windows, and handles the desktop parts natively: its own title bar, tray icon, media keys and the Windows media flyout, an always-on-top mini player, and memory trimming while hidden.

Measured (process tree, playing): ~430 MB private memory and ~4% of one CPU core with the window open, ~190 MB of RAM in the tray, about 2 s to start, a ~3 MB installer. Details in [docs/DESIGN.md](docs/DESIGN.md).

## What it does and does not do

- Loads the service's **official website**, unmodified. You sign in on the site itself; the app never sees or stores your credentials.
- Reads the current track from the page only to show it in the media flyout and tray.
- Does **not** block or alter ads, download or extract audio, use private APIs, or bypass any protection. Some features may require a subscription to the service, exactly as in a browser.

## Features

- A start screen that checks for updates (and installs a newer release) while the player loads, so the window never opens on a blank page.
- Native title bar in the album's colors, with the current track; a mini player that stays on top.
- Tray icon, close to the tray with memory trimmed, media keys and the Windows media flyout, global shortcuts, track toasts while hidden.
- Discord Rich Presence and Last.fm scrobbling (with an offline queue).
- Time-synced lyrics from LRCLIB in the site's Lyrics tab: lines (or words) light up as they are sung, cross-fade into each other, and the list scrolls slowly in time with the song.
- Floating lyrics: the current line over the desktop, click-through, word by word when the lyrics have word times.
- Share lyrics as a picture card: pick any lines (skip the ones you don't want), choose a look, copy or save it.
- Previous / play-pause / next on the taskbar thumbnail and the track's progress on the taskbar button.
- A tray menu in the app's look, with the track and its controls.
- Listening stats, kept only on this PC: listening time, top artists, tracks and albums, when you listen, streaks; CSV export.
- Ambient mode: the blurred, glowing cover behind the site.
- A privacy filter for third-party analytics (never ads).
- Every feature is on out of the box and is a plugin you can switch off in the settings window. Animations follow those switches, not Windows' animation effects, and pause while the window is hidden, minimized or in the background.

What has not been checked yet is listed in [docs/DESIGN.md](docs/DESIGN.md#not-verified-yet).

## Install

Download `feathertune_<version>_x64-setup.exe` from the [releases](https://github.com/bps2414/feathertune/releases) and run it: per user, no admin, Windows 10/11. Uninstalling asks whether to also delete your data (sign-in, `config.toml`); it is kept unless you tick the box.

## Build

Windows 10/11 with the WebView2 runtime and a Rust toolchain (stable, MSVC).

```powershell
cd app
cargo build --release
.\target\release\feathertune.exe
```

`cargo check` is the fast loop while editing; `cargo build --profile dist` is the slow, fully optimized build meant for releases. Installer: `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`.

## Configuration

`%LOCALAPPDATA%\feathertune\config.toml` (created on first run with the plugin list, outside the repository; an existing file keeps the values written in it). The settings window (the gear on the title bar, or tray menu > Settings) edits everything below and keeps your comments; editing by hand works too. Everything is optional:

```toml
[plugins]                    # every optional feature; the settings window shows its description and cost
launcher = true              # start screen: checks GitHub for a newer release, installs it, waits for the player
controls = true              # page module that presses the player's buttons
tracker_filter = true        # blocks third-party analytics requests; never ads (docs/DESIGN.md)
track_notifications = true   # toast on track change, only while the window is hidden
global_shortcuts = true
discord = true               # also needs client_id below
lastfm = true                # also needs your account connected
mini_player = true
animated_title_bar = true     # slow color drift; pauses when hidden, minimized or in the background
album_color = true            # title bar, mini player and site take the cover's colors (once per track)
synced_lyrics = true          # LRCLIB time-synced lyrics in the Lyrics tab, scrolling in time; one request per track, cached
ambient_mode = true           # blurred, glowing cover behind the site
taskbar_controls = true       # buttons on the taskbar thumbnail, progress on the taskbar button
themed_tray_menu = true       # tray menu in the app's look; false: the Windows menu
desktop_lyrics = true         # floating lyrics over the desktop (open it from the tray)
share_lyrics = true           # lyrics picture cards (open it from the tray)
listening_stats = true        # history in stats/plays.jsonl in the profile, shown in the Stats tab

[shortcuts]                  # global; "" turns one off
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # opens/closes the mini player

[discord]
client_id = ""               # your own Discord application id

[lastfm]
api_key = ""                 # optional: release builds carry their own key; these values take priority
api_secret = ""

[browser]
disable_features = []        # extra Chromium features to turn off (experiments, see docs/DESIGN.md)
```

The old keys `[injections] controls`, `[privacy] tracker_filter` and `[notifications] track_change` are still read; the settings window replaces them with the `[plugins]` entry. "Start with Windows", "Connect Last.fm" and the mini player are also in the tray menu. Last.fm sign-in happens in your browser; the session key is kept in the Windows Credential Manager, never in a file. A local build without a Last.fm key needs `api_key` and `api_secret` here.

## Disclaimer

This is an independent, unofficial project. It is not affiliated with, endorsed by or sponsored by any music streaming service or its owners. All trademarks belong to their respective owners. You are responsible for using it in line with the terms of the service you sign in to.

No license has been chosen yet, so all rights are reserved for now.
