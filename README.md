# feathertune

*Português: [README.pt-BR.md](README.pt-BR.md)*

> **Work in progress, experimental.** This is a personal project that I am still testing heavily. Things break, change or get rewritten without notice. There are no releases, no installer and no support yet. Use it at your own risk.

A lightweight Windows desktop shell for a web music player, written in Rust with Tauri 2. Instead of bundling Chromium (like Electron apps do) it uses the WebView2 runtime that already ships with Windows, and handles the desktop parts natively: tray icon, media keys and the Windows media flyout, and memory trimming while hidden.

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
| 3. Integrations (single instance, autostart, shortcuts, Discord, Last.fm) | planned |
| 4. Slimming and installer | planned |

## Build

Windows 10/11 with the WebView2 runtime and a Rust toolchain (stable, MSVC).

```powershell
cd app
cargo build --release
.\target\release\feathertune.exe
```

`cargo check` is the fast loop while editing; `cargo build --profile dist` is the slow, fully optimized build meant for releases.

## Disclaimer

This is an independent, unofficial project. It is not affiliated with, endorsed by or sponsored by any music streaming service or its owners. All trademarks belong to their respective owners. You are responsible for using it in line with the terms of the service you sign in to.

No license has been chosen yet, so all rights are reserved for now.
