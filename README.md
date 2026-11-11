<p align="center">
  <img src="site/public/favicon.svg" alt="feathertune logo" width="96" height="96">
</p>

<h1 align="center">feathertune</h1>

<p align="center"><strong>Your music, as a real Windows app.</strong><br>A ~4.5 MB installer, no bundled Chromium, word-by-word synced lyrics, a mini player and media keys.</p>

<p align="center">
  <a href="https://github.com/bps2414/feathertune/releases/latest"><strong>Download</strong></a> ·
  <a href="https://bps2414.github.io/feathertune/">Website</a> ·
  <a href="#features">Features</a> ·
  <a href="CHANGELOG.md">Changelog</a> ·
  <a href="README.pt-BR.md">Português</a>
</p>

<p align="center">
  <a href="https://github.com/bps2414/feathertune/releases/latest"><img src="https://img.shields.io/github/v/release/bps2414/feathertune?label=release&color=24292f" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/Windows-10%20%7C%2011-24292f" alt="Windows 10 and 11">
  <img src="https://img.shields.io/badge/Rust-Tauri%202-24292f" alt="Rust and Tauri 2">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-24292f" alt="MIT license"></a>
</p>

> **Stable for daily use.** A personal project, used every day and updated through GitHub releases; the app installs new versions on its own. There is no formal support.

## Why feathertune

- **Light.** It uses the WebView2 runtime that already ships with Windows instead of bundling Chromium like Electron apps do. Measured (process tree, playing, at the start of a session): ~510 MB private memory with the window open (its CPU depends on the track: 4–13% of one core), ~225 MB of RAM and ~4% of one core in the tray, about 2 s to start, a ~4.5 MB installer. Over a long session in the tray the memory still grows (being looked into). Details in [docs/DESIGN.md](docs/DESIGN.md).
- **A desktop app, not a browser tab.** Its own title bar in the album's colors, tray icon, media keys and the Windows media flyout, an always-on-top mini player, and memory trimming while hidden.
- **Lyrics that follow the voice.** Word-by-word synced lyrics from four open sources, translated line by line, also floating over the desktop.
- **The official website, untouched.** You sign in on the site itself. Ads are never blocked or altered, and audio is never downloaded.

A lightweight Windows desktop shell for a web music player, written in Rust with Tauri 2.

## What it does and does not do

- Loads the service's **official website**, unmodified. You sign in on the site itself; the app never sees or stores your credentials.
- Reads the current track from the page only to show it in the media flyout and tray.
- Does **not** block or alter ads, download or extract audio, use private APIs, or bypass any protection. Some features may require a subscription to the service, exactly as in a browser.

## Features

**A desktop app, not a tab**
- A start screen that checks for updates (and installs a newer release) while the player loads, so the window never opens on a blank page.
- Native title bar in the album's colors, with the current track; a mini player that stays on top.
- Tray icon, close to the tray with memory trimmed, media keys and the Windows media flyout, global shortcuts, track toasts while hidden.
- Previous / play-pause / next on the taskbar thumbnail and the track's progress on the taskbar button.
- A tray menu in the app's look, with the track and its controls.
- Back and forward buttons on the title bar.
- Precise volume: the mouse wheel over the player changes the volume by 2% (10% with Shift), with optional global volume keys.

**Lyrics**
- Time-synced lyrics in the site's Lyrics tab from four open sources (Better Lyrics, Unison, lrcmux, LRCLIB), preferring lyrics timed word by word: lines and words light up as they are sung (word times are estimated from the song's pace when no source has them), cross-fade into each other, and the list scrolls slowly in time with the song.
- Lyrics translation: a line-by-line translation under each lyric line (into Windows' language by default), the source's own when it has one, else a machine translation made for lyrics; also on the floating lyrics and the mini player.
- Floating lyrics: the current line over the desktop, click-through, word by word when the lyrics have word times.
- Share lyrics as a picture card, from a title bar button: pick any lines (skip the ones you don't want), choose a look, copy or save it.
- The mini player can show the lyrics too: the current line, word by word, and the next one.

**Sound**
- Audio on this PC only, on the page's own sound, every effect bypassed while an ad plays: a 10-band equalizer, a compressor, crossfade between tracks, both playing at once during the fade (with an optional echo out), skipping silence at the start and end of a track, and a visualizer under the artwork.
- Track analysis: the tempo, key and loudness outline of each track, measured the first time it plays; only those numbers are kept, never audio.
- Mixes: per playlist and per pair of tracks, an automatic or hand-edited transition (out and in points, length, curve, echo, filter, bass swap, tempo) in the Mix section of the settings window's Experimental tab. When the account allows it, the start of the next track plays in a hidden second voice so the two overlap; otherwise the track fades out and the next one fades in.

**Your listening**
- Discord Rich Presence and Last.fm scrobbling (with an offline queue).
- Listening stats, kept only on this PC: listening time, top artists, tracks and albums, when you listen, streaks; CSV export.
- Ambient mode: the blurred, glowing cover behind the site.
- Song mode: picks the song version when a track also has a video, so the video is not decoded.
- Skips tracks that start already marked "dislike" (never during an ad).

**Light and private**
- Game mode: a title bar button that turns off at once the features measured as the costliest (synced lyrics in the page, ambient mode, floating and share lyrics windows), and brings them back with a second click.
- A privacy filter for third-party analytics (never ads).
- Export diagnostics (Settings > Advanced): a text file to send when something goes wrong.
- Every feature is on out of the box and is a plugin you can switch off in the settings window, except the experimental ones (crossfade, mixes, skip silence, track analysis, phone remote), which start off in its Experimental tab. After an update the settings open on what changed. Animations follow those switches, not Windows' animation effects, and pause while the window is hidden, minimized or in the background.

**Beyond the PC**
- Phone remote (highly experimental preview): an Android app, paired once with a QR code confirmed on the PC, shows what plays, the artwork and the queue, and controls play/pause, next/previous, seek, live volume, tap-to-play, queue editing (play next, remove) and playing a playlist of your library. Local network only, end-to-end encrypted (Noise Protocol), never during an ad except pause. The APK is attached to each release; the app is in [mobile/](mobile/README.md), the protocol in [docs/REMOTE.md](docs/REMOTE.md).
- Extensions: import plugins made by others (or yourself) from a folder, a .zip or a single .js or .css file, in the settings window's Extensions tab. They are your own choice: the app does not review them. How to write one: [docs/EXTENSIONS.md](docs/EXTENSIONS.md).

What has not been checked yet is listed in [docs/DESIGN.md](docs/DESIGN.md#not-verified-yet).

## Install

Download `feathertune_<version>_x64-setup.exe` from the [releases](https://github.com/bps2414/feathertune/releases) and run it: per user, no admin, Windows 10/11. Uninstalling asks whether to also delete your data (sign-in, `config.toml`); it is kept unless you tick the box.

The phone remote's Android APK (`feathertune-remote-<version>.apk`) is attached to every release (rebuilt only when it changes). Turn on Phone remote in the settings' Experimental tab on the PC, then scan the QR code from the phone app.

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

<details>
<summary>Every option, with comments</summary>

```toml
[plugins]                    # every optional feature; the settings window shows its description and cost
launcher = true              # start screen: checks GitHub for a newer release, installs it, waits for the player
controls = true              # page module that presses the player's buttons
tracker_filter = true        # blocks third-party analytics requests; never ads (docs/DESIGN.md)
track_notifications = true   # toast on track change, only while the window is hidden
global_shortcuts = true
discord = true               # a release has an id; otherwise needs client_id below
lastfm = true                # also needs your account connected
mini_player = true
animated_title_bar = true     # slow color drift; pauses when hidden, minimized or in the background
album_color = true            # title bar, mini player and site take the cover's colors (once per track)
synced_lyrics = true          # time-synced lyrics in the Lyrics tab, scrolling in time; a few requests per new track, cached
lyrics_translation = true     # a translation under each lyric line; one request per new track, cached
lyric_effects = true          # dots that fill up over the intro and long breaks of the lyrics
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
crossfade = false             # experimental (off by default); also the mixes and the transition voice
track_analysis = false        # experimental; tempo, key and loudness outline in audio-facts/ in the profile
skip_silence = false          # experimental
lyric_colors = false          # experimental; a sung word named after a color (red, azul...) glows in it, verses in the album's colors
lyric_pulse = false           # experimental; the sung line's glow breathes on the beat (needs track_analysis)
remote = false                # experimental; phone remote: opens a TCP port for paired phones only (see [remote])
visualizer = true

[shortcuts]                  # global; "" turns one off
play_pause = "Ctrl+Alt+Space"
next = "Ctrl+Alt+Right"
previous = "Ctrl+Alt+Left"
mini_player = ""             # opens/closes the mini player
volume_up = ""               # volume up by 2% (5% without precise_volume)
volume_down = ""

[discord]
client_id = ""               # a release has its own; set yours to use it instead (or in a local build)

[lastfm]
api_key = ""                 # optional: release builds carry their own key; these values take priority
api_secret = ""

[lyrics]
providers = ["betterlyrics", "unison", "lrcmux", "lrclib"]  # synced lyrics sources; word-timed ones win, then this order
estimate_words = true        # guess word times when a source times whole lines only
offset_ms = 0                # moves the lyrics in time; positive: later
translate_to = "auto"        # translation language: auto (Windows' language), off, pt, en, es, fr, de, it, ja, ko
romanize = false             # the source's romanization (e.g. Japanese, Korean) under its line
size = "medium"              # Lyrics tab text: small, medium, large
align = "left"               # left, center or right
duets = false                # a duet by singer: the first on the left, the second on the right
glow = "normal"              # off (lightest), normal, strong

[audio]                      # the Audio tab; mixes are in mixes/<playlist>.json in the profile (Experimental tab)
eq_preset = "flat"           # flat (out of the sound path), bass, vocal, treble, loudness, custom
eq_bands = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]  # custom: dB from -12 to 12, 31 Hz ... 16 kHz
compressor = "none"          # none (out of the sound path), soft, night, strong
crossfade_ms = 6000          # 0 to 12000; 0: off
curve = "equal_power"        # linear, equal_power, s_curve, exponential
fade_in_ms = 1500            # 0 to 5000, into the next track
echo_out = false             # an echo on the tail of the track that fades out
visualizer = "bars"          # bars, wave

[remote]                     # the phone remote (Experimental tab)
port = 47652                 # TCP, 1024 to 65535; local network sources only
name = "feathertune"         # what paired phones call this PC (never the Windows name)

[ui]
language = "auto"            # auto (Windows' language: Portuguese gets pt-BR, anything else English), en, pt-BR; applies after a restart

[browser]
disable_features = []        # extra Chromium features to turn off (experiments, see docs/DESIGN.md)
```

</details>

The old keys `[injections] controls`, `[privacy] tracker_filter` and `[notifications] track_change` are still read; the settings window replaces them with the `[plugins]` entry. "Start with Windows", "Connect Last.fm" and the mini player are also in the tray menu. Last.fm sign-in happens in your browser; the session key is kept in the Windows Credential Manager, never in a file. A local build without a Last.fm key needs `api_key` and `api_secret` here.

## Contributing

Pull requests are welcome; open an issue first for anything big. The full guide (rules, build discipline, what is not accepted, how review works) is [CONTRIBUTING.md](CONTRIBUTING.md). Security problems: [SECURITY.md](SECURITY.md).

## Disclaimer

This is an independent, unofficial project. It is not affiliated with, endorsed by or sponsored by any music streaming service or its owners. All trademarks belong to their respective owners. You are responsible for using it in line with the terms of the service you sign in to.

Licensed under the [MIT License](LICENSE).

---

<p align="center">
  <a href="https://star-history.com/#bps2414/feathertune&Date"><img src="https://api.star-history.com/svg?repos=bps2414/feathertune&type=Date" alt="Star history" width="560"></a>
</p>

<p align="center">If feathertune made your music feel at home on Windows, a ★ on the repo helps other people find it.</p>
