# feathertune: design

*Português: [DESIGN.pt-BR.md](DESIGN.pt-BR.md). Keep both files in sync.*

How the app is built, why, what it costs, and what is still unverified. The history of how it got here is in the Git log.

## Approach

The app opens the music service's **official website** in a lean WebView2 window and does everything Windows-side in Rust (Tauri 2). It uses no private API and no official data API, so it does not break when the service changes its protocol, and sign-in and subscriptions work exactly as in a browser.

| Approach | Weight | Breaks? | Problem |
|---|---|---|---|
| Native client on private APIs | ~100 MB | yes, all the time | Constant race against protocol and anti-abuse changes, and against the terms of service |
| Official data API + embedded player | ~Chromium | no | Tiny quotas; its policies forbid background or audio-only playback |
| **Official website in a lean WebView2** | ~450 MB | no (it is the site itself) | Still Chromium inside |

A custom engine was ruled out too: the page is most of the weight (empty WebView2 ~121 MB private, the site playing ~450 MB), CEF and Electron ship their own Chromium, Servo has no complete Media Source Extensions support yet, there is no embeddable Gecko, and a native client needs the private API.

**Ground rules** (permanent):
- The site is unmodified in what it plays. No private APIs, stream extraction, downloading or DRM handling.
- Ads are never blocked, hidden or altered.
- No user-agent spoofing and no automation flags (they also trigger sign-in blocks).
- The app never types or stores credentials; sign-in happens on the site.

## Architecture

```
feathertune.exe (Rust, Tauri 2)                      WebView2 (the system Chromium)
├─ native title bar and mini player (Direct2D)       ├─ site webview: the official site
├─ tray, close = hide (memory Low)                   │   └─ injected modules: read the player state,
├─ player state  ◄──── one-way bridge ───────────────┘      press its buttons, presentation CSS
├─ media keys / Windows media flyout (SMTC)
├─ single instance, autostart, global shortcuts      └─ settings webview: our own page on a
├─ Discord, Last.fm, track toasts, LRCLIB lyrics         custom scheme, no access to the site
├─ album colors (WIC decode of the cover)
├─ taskbar buttons, themed tray menu, floating lyrics (Direct2D)
├─ listening stats (a file in the profile)
└─ tracker filter (privacy lists only)
```

- **Window:** `main` has no system frame; Rust paints a 32 DIP strip (app name, current track, mini player and settings buttons, caption buttons) and answers `WM_NCHITTEST` natively, so snap layouts, dragging and resizing are Windows' own. The site is a child webview below the strip.
- **Memory:** hidden (tray or mini player) sets WebView2's `MemoryUsageTargetLevel=Low`; shown sets `Normal`. Playback continues.
- **Bridge:** the `core` injection observes `navigator.mediaSession` and the `<video>` and sends Rust title, artist, artwork, time and state, only on track change, play/pause and seek plus a correction every 15 s (sending 1×/s cost ~3% CPU); Rust extrapolates the position. Rust treats the payload as untrusted: it validates it and never acts on it. Rust → page commands click the player's buttons, so a site redesign breaks about three CSS selectors, not a protocol.
- **Data to the page:** modules get data only, as `window.__feathertune.<module>.set(JSON.parse("…"))`, never code built from strings. The last data of each module is re-sent after a reload.
- **Chromium flags:** every feature goes in the single `--disable-features` (Chromium only honors the last one).

### Security

- The site webview's capability (`capabilities/player-bridge.json`) grants only `player_state`, only to the `site` webview and only for the site's origin. `frontendDist` is not used: with it, Tauri treats the remote site as a local origin.
- The settings webview (`capabilities/settings-window.json`) is `local: true` and grants only the four `settings_*` commands. The fourth, `settings_stats`, was added for the Stats tab: it returns a summary of the listening history, which is the user's own data on this PC, to our own page only; it takes a period and a time zone offset, both checked, and changes nothing. Tauri matches capabilities by label **and** origin, so neither page can reach the other's commands; neither has `core:*` permissions. The commands also check the caller's label.
- Custom schemes are registered for every webview (a Tauri limitation), so the scheme handler answers 403 to anything but the settings webview. The settings page has a strict CSP and builds its DOM with `textContent` only.
- Navigation is pinned to the site and sign-in domains; everything else opens in the default browser.

## Plugins

Every optional feature is a plugin in `app/src/plugins.rs`, with a switch under `[plugins]` in `config.toml` and in the settings window. Every plugin is on by default; the switch is the control. Nothing is loaded from a DLL or downloaded.

| Plugin | Default | Cost |
|---|---|---|
| `launcher` (start screen, update check) | on | not measured: one small request to GitHub per start |
| `controls` (presses the player's buttons) | on | none measurable |
| `tracker_filter` | on | saves ~16 MB WS and some CPU |
| `track_notifications`, `global_shortcuts` | on | none measurable |
| `discord`, `lastfm` | on (need an app id / an account) | not measured: one idle thread each |
| `mini_player` | on | nothing while closed |
| `animated_title_bar` | on | not measured yet |
| `album_color` | on | +1 MB private, inside the noise |
| `synced_lyrics` (LRCLIB) | on | ~+5 MB, inside the spread (one run); the cross-fade and scroll not measured yet |
| `ambient_mode` | on | ~+25 MB private, +15 MB WS (one run) |
| `taskbar_controls` | on | not measured yet: a progress update every 1–5 s while playing with the window shown |
| `themed_tray_menu` | on | not measured yet: nothing while closed |
| `desktop_lyrics` | on (opened from the tray) | not measured yet: nothing while closed; a repaint per line or word change |
| `listening_stats` | on | not measured yet: one idle thread, one line appended per track |

**Tracker filter:** WebView2 `WebResourceRequested` and the `adblock` crate with cosmetic rules dropped, so it can only block requests. It only sees scripts, images, XHR/fetch, pings and "other" (never documents, frames, media, fonts or stylesheets), and skips the player's domains, every Google domain and Google's ad-delivery domains before the list is asked, so even an ad list dropped in by mistake changes nothing there. The bundled list (`app/filters/privacy.txt`) has only third-party analytics; on this site it blocks one host.

**Last.fm:** desktop auth (`auth.getToken` → browser → `auth.getSession`, polled); the session key lives in the Windows Credential Manager. CI release builds embed the maintainer's API key from the repository secrets `FEATHERTUNE_LASTFM_API_KEY` / `_SECRET` (`option_env!`); keys in `config.toml` take priority. The scrobble point counts played time only; failed scrobbles wait in `lastfm-queue.json` (retried every 5 min, dropped after 14 days).

**Synced lyrics:** one `GET https://lrclib.net/api/get` per track (title, artist, album, duration), cached in `lyrics-cache/` in the profile; shown in the site's Lyrics tab only when LRCLIB has synced lines. Entries in Enhanced LRC (a `<mm:ss.xx>` before each word) light up word by word, on the page's own `timeupdate` (about 4 per second, no extra timer); most entries are timed by line only. Lines cross-fade (the old one dims slowly, the new one comes in faster). While the song plays and the window is in front, an animation-frame loop scrolls the list in time: the view parks on the current line and moves on to the next one around the switch (from 40% of the ending line, at most 1.6 s, before it, to 45% of the new one, at most 1.8 s, after it, eased), so a line that ends is still a little short of halfway and short lines keep the view moving; it eases to a new place after a seek, a click or a manual scroll, and writes the scroll only when it moves by a whole device pixel; it also drives the line and word changes then. Otherwise each new line scrolls smoothly to the middle. Other sources with word timing (Musixmatch, NetEase, Apple Music, or proxies of them) are private APIs and stay out.

**Floating lyrics:** a native layered window (Direct2D into a 32-bit bitmap, `UpdateLayeredWindow`) with the current line, lit word by word when the entry has word times, and the next one dimmer, outlined so it reads on any background. Its lines come from Rust (`lyrics::Lyrics`), so it works with `synced_lyrics` off; the time is `position_now()` with the page's 250 ms lead. A one-shot timer is set to the next line or word, and a 60 fps timer runs only for the 320 ms cross-fade. Locked it is click-through (`WS_EX_TRANSPARENT`); unlocked (tray) it shows a panel to drag and a lock button. Hidden while paused or without synced lyrics. Open state and position live in `desktop-lyrics.json` in the profile.

**Taskbar:** `ITaskbarList3` from a subclass of the main window: previous / play-pause / next on the thumbnail (white glyph icons drawn once with Direct2D at the small icon size of the DPI) and the track's progress on the taskbar button, yellow while paused. The buttons are added on each "TaskbarButtonCreated" (the window shown from the tray, Explorer restarted). Progress updates once per 1% of the track (1–5 s) while playing with the window shown.

**Themed tray menu:** a native popup in the album's ink and glows (or the system colors) with the track and its controls on top and the tray items below; it closes on focus loss, Esc, a click outside or after an item. It is not exposed to screen readers as a menu, which the Windows menu is: switching the plugin off brings that one back.

**Listening stats:** each listen of at least 10 s of played time (counted like the scrobble point) is one JSON line in `stats/plays.jsonl` in the profile; a play is half the track or 30 s, less is a skip. The Stats tab of the settings window asks `settings_stats` for a period (7 days, 30 days, 12 months, all) and gets totals, a daily or monthly series, an hour × weekday map, streaks and the top artists, tracks and albums, computed in Rust from the whole file. Export writes a CSV to Downloads; "clear history" deletes the file. The listen in progress is written on a track change and on quit or restart from the app; a crash or a Windows shutdown loses it.

**Launcher:** on an interactive start (not `--hidden`, `--mini-player` or `--measure`) a small native start screen (Direct2D) opens first and says what the app is doing. The main window is created hidden, so the site loads meanwhile; the window is shown when the update check is done and the site has drawn its first content (a probe on the page, at most 8 s after `load`, 20 s overall), never on a blank page. The update check is one `GET` of the repository's latest release on the GitHub API. A newer release's installer is downloaded, its SHA-256 compared with the one GitHub lists for the asset (the installer is not signed, so a missing or different hash stops the update), and run silently (`/S /R /UPDATE`: it closes the app, replaces it and starts it again). Only an installed copy updates itself (the exe is in the profile folder, where the installer puts it); a build from `target/` never does. Any failure, offline included, just starts the current version.

## UI performance rules

Draw only on change. Any animation has a plugin switch and runs on one timer only while there is something on screen to move: it stops when the window is hidden, minimized or in the background, and resumes where it stopped. Windows' animation effects do not turn animations off; the plugin switches do (the page gets `--force-prefers-no-reduced-motion`, so the site and the page modules animate too). Invalidate only the region that changes. Keep GPU resources (brushes, layouts, bitmaps) between frames. Features are on by default; measure each one and write its cost in the plugin list, so the switch is an informed choice.

## Measurements

Criterion: `Win32_Process`, sum of the process tree; private = `PrivatePageCount`, WS = `WorkingSetSize`; CPU as % of one core over 20 s. Signed in, 1280×800, one fixed track playing, 3 runs, median (min–max).

| Build (2026-10-20) | Scenario | CPU | Private | WS | Start to player bar |
|---|---|---|---|---|---|
| without the native shell, `dist` | window open | 3.3% (3.1–3.4) | 403 MB | 733 MB | 1.9 s |
| without the native shell, `dist` | in the tray | 3.3% | 411 MB | **188 MB** | 2.0 s |
| native shell, `dist` | window open | 4.5% (4.1–5.5) | 430 MB | 761 MB | 1.9 s |
| native shell, `dist` | mini player, main window hidden | 4.0% | 442 MB | 218 MB | 2.0 s |
| current defaults, `release` | window open | 4.4% (3.1–4.5) | 433 MB | 765 MB | 1.9 s |

| | Electron client (reference, earlier measurement) | Target |
|---|---|---|
| Window open | 16–32% CPU, 672 MB private, 1009 MB WS | ≤ 4% CPU, ≤ 480 MB private, ≤ 750 MB WS |
| In the tray | same as open | ≤ 260 MB WS |
| Installer | ~100 MB | a few MB (feathertune: ~3 MB) |

- The native-shell rows are above the earlier ones, but there was no same-day baseline, so it is not known yet how much comes from the shell and how much from the site or the WebView2 runtime changing. Open and WS are slightly over target.
- Kept from the experiments: no Cast discovery (`MediaRouter`, `DialMediaRouteProvider`: −12 MB WS) and the tracker filter (−16 MB WS). Tried and dropped: no spare renderer, no prerender, no optimization hints, a CSS "no animations" module, a full EasyPrivacy list. Not tried on purpose: site isolation off, `--disable-component-update`, `--disable-gpu` (CPU 15%), V8 lite mode (CPU up), anything ad-related.
- Where the RAM goes (playing, private): main process ~40 MB + ~4 MB for the Rust exe, GPU ~190 MB, renderer ~180 MB, network/audio/storage ~33 MB. What remains is Chromium plus the page itself, which no wrapper can cut.

### Reproducing

`measure.ps1` measures any exe (default: the release build) on the real profile, so it measures signed in. Close the app first: two instances share the browser process. `--measure` makes the app print `player_bar_ms` and, every 10 s, the `<video>` time and sign-in state. Output goes to `results/` (gitignored: it has local paths and listening history).

```powershell
cd app; cargo build --release; cd ..
$a = '--measure', '<track url>'
1..3 | % { ./measure.ps1 -Name open-$_ -ArgumentList $a }                          # window open
1..3 | % { ./measure.ps1 -Name tray-$_ -ArgumentList $a -HideAfter 25 -Wait 50 }   # in the tray
./measure.ps1 -Name mini -ArgumentList $a -MiniAfter 25 -Wait 50                     # mini player
./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60
./median.ps1 -Prefix open-
```

`-HideAfter` sends WM_CLOSE (the app goes to the tray); `-MiniAfter` runs `feathertune.exe --mini-player` in the running instance; `-Exe app/target/dist/feathertune.exe` measures the fully optimized `dist` build (the installer's).

## Installer

Tauri's NSIS bundler, per user (no admin), installed to `%LOCALAPPDATA%\feathertune`, the same folder as the profile. The WebView2 bootstrapper is downloaded only where the runtime is missing. English and Brazilian Portuguese. The Start menu shortcut carries the AppUserModelID `dev.feathertune.desktop` (the one the process sets, so the media flyout shows the app's name). Uninstalling keeps the profile unless the "delete data" box is ticked; then `installer-hooks.nsh` also removes the profile and the Last.fm credential.

Pushing a `v*` tag runs the CI `release` job, which builds with `--profile dist` and attaches the installer to a GitHub release.

## Not verified yet

- With a real mouse: snap layouts, dragging, resizing and maximize/restore, at 150% scaling and on a second monitor; typing in the site after switching windows.
- Discord Rich Presence and a real Last.fm sign-in and scrobble, including on a CI build with the embedded key.
- "Start with Windows" writing its Run entry; the track toast being seen; the 1 h tray test.
- The "delete data" uninstall path.
- The 0.7 s color flow between covers, light-mode Windows, an LRCLIB miss, and a reload keeping the theme and lyrics.
- A same-day baseline to tell whether the native shell costs memory and CPU, and the `animated_title_bar` plugin measured on and off.
- The lyrics cross-fade and in-time scroll in the running app (smoothness, manual scroll and seeks) and their CPU cost; the new parked scroll's timings.
- In the running app: the taskbar buttons and progress (also after Explorer restarts and when running elevated); the themed tray menu (position on each taskbar edge, closing on a click outside, keyboard, 150% scaling); the floating lyrics (readability, word timing against the site, dragging, a second monitor); the Stats tab with a real history and the CSV export.
- The cost of the four new plugins.

## Ideas

| Idea | Type |
|---|---|
| More looks (fonts, compact mode, large artwork) | CSS |
| Share lyrics as an image: a native window to pick any lines (skipping some) and render a card with the cover | Rust (Direct2D, WIC) |
| Equalizer and compressor (Web Audio on the `<video>`): first decide whether it breaks "never touch what the player plays" | JS |

## References

- WebView2: [process model](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html)
- Embedded sign-in in WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
- Last.fm [desktop auth](https://www.last.fm/api/desktopauth); [LRCLIB API](https://lrclib.net/docs)
