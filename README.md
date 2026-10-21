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
- Time-synced lyrics in the site's Lyrics tab from four open sources (Better Lyrics, Unison, lrcmux, LRCLIB), preferring lyrics timed word by word: lines and words light up as they are sung (word times are estimated from the song's pace when no source has them), cross-fade into each other, and the list scrolls slowly in time with the song.
- Floating lyrics: the current line over the desktop, click-through, word by word when the lyrics have word times.
- Share lyrics as a picture card, from a title bar button: pick any lines (skip the ones you don't want), choose a look, copy or save it.
- The mini player can show the lyrics too: the current line, word by word, and the next one.
- Previous / play-pause / next on the taskbar thumbnail and the track's progress on the taskbar button.
- A tray menu in the app's look, with the track and its controls.
- Listening stats, kept only on this PC: listening time, top artists, tracks and albums, when you listen, streaks; CSV export.
- Ambient mode: the blurred, glowing cover behind the site.
- Back and forward buttons on the title bar.
- Song mode: picks the song version when a track also has a video, so the video is not decoded.
- Precise volume: the mouse wheel over the player changes the volume by 2% (10% with Shift), with optional global volume keys.
- Skips tracks that start already marked "dislike" (never during an ad).
- A privacy filter for third-party analytics (never ads).
- Audio on this PC only, on the page's own sound, every effect bypassed while an ad plays: a 10-band equalizer, a compressor, crossfade between tracks (with an optional echo out), skipping silence at the start and end of a track, and a visualizer under the artwork.
- Track analysis: the tempo, key and loudness outline of each track, measured the first time it plays; only those numbers are kept, never audio.
- Mixes: per playlist and per pair of tracks, an automatic or hand-edited transition (out and in points, length, curve, echo, filter, bass swap, tempo) in the settings window's Mix tab. When the account allows it, the start of the next track plays in a hidden second voice so the two overlap; otherwise the track fades out and the next one fades in.
- Every feature is on out of the box and is a plugin you can switch off in the settings window. Animations follow those switches, not Windows' animation effects, and pause while the window is hidden, minimized or in the background.

What has not been checked yet is listed in [docs/DESIGN.md](docs/DESIGN.md#not-verified-yet).

## Install

Download `feathertune_<version>_x64-setup.exe` from the [releases](https://github.com/bps2414/feathertune/releases) and run it: per user, no admin, Windows 10/11. Uninstalling asks whether to also delete your data (sign-in, `config.toml`); it is kept unless you tick the box.

What changed in each version: [CHANGELOG.md](CHANGELOG.md).

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
synced_lyrics = true          # time-synced lyrics in the Lyrics tab, scrolling in time; a few requests per new track, cached
ambient_mode = true           # blurred, glowing cover behind the site
taskbar_controls = true       # buttons on the taskbar thumbnail, progress on the taskbar button
themed_tray_menu = true       # tray menu in the app's look; false: the Windows menu
desktop_lyrics = true         # floating lyrics over the desktop (open it from the tray)
share_lyrics = true           # lyrics picture cards (open it from the tray)
listening_stats = true        # history in stats/plays.jsonl in the profile, shown in the Stats tab
navigation = true             # back and forward buttons on the title bar
song_mode = true              # picks the song version of a track that also has a video
precise_volume = true         # mouse wheel over the player: volume by 2% (Shift: 10%)
skip_disliked = true          # skips a track that starts marked "dislike"
equalizer = true              # audio plugins: one Web Audio graph on the page, bypassed during ads
compressor = true
crossfade = true              # also the Mix tab and the transition voice
track_analysis = true         # tempo, key and loudness outline in audio-facts/ in the profile
skip_silence = true
visualizer = true

[shortcuts]                  # global; "" turns one off
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # opens/closes the mini player
volume_up = ""               # volume up by 2% (5% without precise_volume)
volume_down = ""

[discord]
client_id = ""               # your own Discord application id

[lastfm]
api_key = ""                 # optional: release builds carry their own key; these values take priority
api_secret = ""

[lyrics]
providers = ["betterlyrics", "unison", "lrcmux", "lrclib"]  # synced lyrics sources; word-timed ones win, then this order
estimate_words = true        # guess word times when a source times whole lines only
offset_ms = 0                # moves the lyrics in time; positive: later

[audio]                      # the Audio tab; mixes are in mixes/<playlist>.json in the profile (Mix tab)
eq_preset = "flat"           # flat (out of the sound path), bass, vocal, treble, loudness, custom
eq_bands = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]  # custom: dB from -12 to 12, 31 Hz ... 16 kHz
compressor = "none"          # none (out of the sound path), soft, night, strong
crossfade_ms = 6000          # 0 to 12000; 0: off
curve = "equal_power"        # linear, equal_power, s_curve, exponential
fade_in_ms = 1500            # 0 to 5000, into the next track
echo_out = false             # an echo on the tail of the track that fades out
visualizer = "bars"          # bars, wave

[browser]
disable_features = []        # extra Chromium features to turn off (experiments, see docs/DESIGN.md)
```

The old keys `[injections] controls`, `[privacy] tracker_filter` and `[notifications] track_change` are still read; the settings window replaces them with the `[plugins]` entry. "Start with Windows", "Connect Last.fm" and the mini player are also in the tray menu. Last.fm sign-in happens in your browser; the session key is kept in the Windows Credential Manager, never in a file. A local build without a Last.fm key needs `api_key` and `api_secret` here.

## Disclaimer

This is an independent, unofficial project. It is not affiliated with, endorsed by or sponsored by any music streaming service or its owners. All trademarks belong to their respective owners. You are responsible for using it in line with the terms of the service you sign in to.

No license has been chosen yet, so all rights are reserved for now.
