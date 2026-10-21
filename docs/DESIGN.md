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
- The site plays what it serves; the app may shape how it sounds, locally (see Audio). No private APIs, stream extraction, downloading or DRM handling.
- Ads are never blocked, hidden, skipped, shortened, muted, faded or covered; every audio effect is bypassed while one plays.
- Audio is processed and measured only locally, for the listener: never recorded, stored, uploaded or re-encoded, and no stream is ever fetched. Only numbers derived from it (tempo, key, a loudness envelope) are kept, in the profile.
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
├─ Discord, Last.fm, track toasts, lyrics sources        custom scheme, no access to the site
├─ album colors (WIC decode of the cover)
├─ taskbar buttons, themed tray menu, floating lyrics (Direct2D)
├─ listening stats (a file in the profile)
└─ tracker filter (privacy lists only)
```

- **Window:** `main` has no system frame; Rust paints a 32 DIP strip (app name, current track, share lyrics (only while the track has synced lyrics), mini player and settings buttons, caption buttons) and answers `WM_NCHITTEST` natively, so snap layouts, dragging and resizing are Windows' own. The site is a child webview below the strip. When the page goes fullscreen (the player's own button; WebView2's `ContainsFullScreenElementChanged`) the window goes fullscreen with it and the site takes the strip's place; leaving it (or hiding the window, which makes the page exit) restores both. The site is laid out again on show, on a DPI or display change and on fullscreen changes, not only on `WM_SIZE`. The tray menu has "Reload page" as a way out of a stuck page.
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
| `synced_lyrics` (LRCLIB, Better Lyrics) | on | ~+5 MB, inside the spread (one run); the cross-fade and scroll not measured yet |
| `ambient_mode` | on | ~+25 MB private, +15 MB WS (one run) |
| `taskbar_controls` | on | not measured yet: a progress update every 1–5 s while playing with the window shown |
| `themed_tray_menu` | on | not measured yet: nothing while closed |
| `desktop_lyrics` | on (opened from the tray) | not measured yet: nothing while closed; a repaint per line or word change |
| `share_lyrics` | on (opened from the title bar, the tray or settings) | not measured yet: nothing until opened; repaints only on input |
| `listening_stats` | on | not measured yet: one idle thread, one line appended per track |
| `navigation` | on (live) | not measured yet: a repaint of the two buttons when the history changes |
| `song_mode` | on | not measured yet: one attribute observer, one click per track; saves video decoding on tracks that have a video |
| `precise_volume` | on | not measured yet: one wheel listener, nothing runs otherwise |
| `skip_disliked` | on | not measured yet: one check per track |

**Tracker filter:** WebView2 `WebResourceRequested` and the `adblock` crate with cosmetic rules dropped, so it can only block requests. It only sees scripts, images, XHR/fetch, pings and "other" (never documents, frames, media, fonts or stylesheets), and skips the player's domains, every Google domain and Google's ad-delivery domains before the list is asked, so even an ad list dropped in by mistake changes nothing there. The bundled list (`app/filters/privacy.txt`) has only third-party analytics; on this site it blocks one host.

**Last.fm:** desktop auth (`auth.getToken` → browser → `auth.getSession`, polled); the session key lives in the Windows Credential Manager. CI release builds embed the maintainer's API key from the repository secrets `FEATHERTUNE_LASTFM_API_KEY` / `_SECRET` (`option_env!`); keys in `config.toml` take priority. The scrobble point counts played time only; failed scrobbles wait in `lastfm-queue.json` (retried every 5 min, dropped after 14 days).

**Synced lyrics:** the sources of `[lyrics] providers` (settings window, Lyrics tab) are asked all at once, each on a short-lived thread. The first answer with lines is shown at once and a better one replaces it when it comes: lines timed word by word win over lines timed as a whole, then the order of the list (default: Better Lyrics, Unison, lrcmux, LRCLIB); it stops waiting once no source ahead of the winner could still bring word times. The answer is cached in `lyrics-cache/` in the profile: word-timed lines while their source is on; lines timed as a whole and a "not found" for a week and only for the same list of sources, since a source may get word times later. On four test tracks the first lines showed in about 0.1–1.3 s and lrcmux answered in up to 5 s.
- **LRCLIB** (open, community-made): `GET https://lrclib.net/api/get` (title, artist, album, duration). With no synced lines there, `GET /api/search` with the title cleaned of what video titles add ("(Official Video)", "[Lyrics]", "feat. ...", an "Artist - " in front, " - Remastered" behind), then by artist and title only (`q`) if that finds nothing; the result kept is the closest in length (at most 10 s off) whose artist and title match (normalized, equal or one holding the other) and that is not instrumental.
- **Unison** ([open database](https://docs.betterlyrics.org/unison), 120 reads a minute, no key): Better Lyrics' community-synced lyrics, mostly word by word (TTML, or LRC). By the video id, else by title, artist and duration; entries the community hid are skipped, and a different length is a miss.
- **lrcmux** ([public API](https://lrcmux.dev), no key): a lyrics aggregator, asked for word times only (`level=word&strict=true`) and never from the service's own lyrics or Genius (`sources=!ytmusic,!genius`); its word times come mostly from KuGou. Garbled text (U+FFFD) is a miss.
- **Better Lyrics** ([public API](https://docs.betterlyrics.org), open to third-party apps): `GET https://api.betterlyrics.org/getLyrics` (title, artist, album, duration), TTML timed word by word; background vocals are left out, and a different song length is a miss. Without an API key (none are issued now) it answers only for tracks it has cached, so a 401 is a miss. Its lyrics come from Apple Music through that service: we call only the service's documented public API, never Apple's, which is why it is allowed while Apple Music, Musixmatch, NetEase or KuGou called directly (private APIs) stay out. lrcmux is allowed on the same terms.

Lines timed only as a whole (no source had word times) get **estimated word times** (`[lyrics] estimate_words`, on by default): each word weighs its syllables (groups of vowels, Hangul blocks; a Chinese or Japanese character is a word), +0.5 after a comma, and the line is sung at the song's own pace, the first quartile of time per syllable over its lines, plus 15% of whatever time is left before the next line (slow lines use more of it), within 92% of that time, so words do not stretch over an instrumental gap. Against real word times (7 songs, 2411 words) the start of a word is off by 253 ms on average (median 140 ms), against 533 ms for spreading the words over the whole line by length; a word is more than 0.3 s early 12% of the time and late 14%, and one line in five has a word 0.4 s early. An extra weight for a held last word was dropped: it pulled every other word early (one line in three). Checked on English songs only. The page lights estimated words with a softer transition and says so in the credit. `[lyrics] offset_ms` moves every time. Both apply at once to the current track (the lines kept from the source are processed again).

Lines with word times of their own (Better Lyrics, Unison, lrcmux, Enhanced LRC entries of LRCLIB) light up word by word with those times, on the page's own `timeupdate` (about 4 per second, no extra timer). The page puts its box in the tab renderer showing the Lyrics page (there can be more than one, e.g. with an album page open), checked again when any `page-type` changes and on the video's time events. Lines cross-fade (the old one dims slowly, the new one comes in faster). While the song plays and the window is in front, an animation-frame loop scrolls the list in time: the view parks on the current line and moves on to the next one around the switch (from 40% of the ending line, at most 1.6 s, before it, to 45% of the new one, at most 1.8 s, after it, eased), so a line that ends is still a little short of halfway and short lines keep the view moving; it eases to a new place after a seek, a click or a manual scroll, and writes the scroll only when it moves by a whole device pixel; it also drives the line and word changes then. Otherwise each new line scrolls smoothly to the middle. Ruled out: Musixmatch's desktop-app token, the service's own timed lyrics (they need another client's identity), Genius (plain lyrics only, which the site has), Megalobiz (gone, HTTP 410), SimpMusic's lyrics API (its own clients only), and aligning lyrics to the audio (it would need a speech model of hundreds of MB).

**Floating lyrics:** a native layered window (Direct2D into a 32-bit bitmap, `UpdateLayeredWindow`) with the current line, lit word by word when the entry has word times, and the next one dimmer, outlined so it reads on any background. Its lines come from Rust (`lyrics::Lyrics`), so it works with `synced_lyrics` off; the time is `position_now()` with the page's 250 ms lead. A one-shot timer is set to the next line or word, and a 60 fps timer runs only for the 420 ms roll: the next line rises into the current one's place and grows into it while the old line fades out upward and the new next line fades in from below, so two lines never sit on top of each other. Locked it is click-through (`WS_EX_TRANSPARENT`); unlocked (tray) it shows a panel to drag and a lock button. Hidden while paused or without synced lyrics. Open state and position live in `desktop-lyrics.json` in the profile. The line timing and drawing (`lyric_view.rs`) are shared with the mini player: its lyrics button grows it by a band with the current and next line in the window's colors (no outline), kept while paused, clipped to the band and rolled at 60 fps; whether the band is open is saved in `mini-player.json`. Its controls are 30–38 DIP targets (play/pause a 38 DIP disc), and the top ones (lyrics, full window, close) sit on faint glass pills and show what they do in a small label on hover.

**Share lyrics:** a native window (Direct2D) lists the synced lines of the track (from `lyrics::Lyrics`, so it works with `synced_lyrics` off) and previews a card. Any lines can be picked, up to 8, with a click, or a range with Shift-click; lines skipped between two picks show as three dots on the card. Looks: Cover (a soft field of the cover's colors: its center square shrunk to 5×5 and blown up with a cubic filter, with WIC, with the theme's glows and darkened toward the bottom), Ink (the album theme's ink and glows), Dark and Paper. The card has the cover, title and artist on top, a large quote mark, the lines, and a footer with a drawn note and the album; no app name. The album's dominant color is the accent (quote mark, a bar fading down beside the lines, skip dots, the note), the lines have a soft drop shadow, Ink and Dark a vignette, and every look a faint fixed film grain. It is opened by a button in the title bar that is there only while the track has synced lyrics (a native button, so the page gets no new command), from the tray or from the settings window. The card is drawn by one function, scaled into the preview and into a 32-bit bitmap at 3× (1080 px wide); it is opaque, so the clipboard gets it as a DIB and as "PNG", and Save writes a PNG (WIC) to Downloads. Nothing is uploaded anywhere.

**Taskbar:** `ITaskbarList3` from a subclass of the main window: previous / play-pause / next on the thumbnail (white glyph icons drawn once with Direct2D at the small icon size of the DPI) and the track's progress on the taskbar button, yellow while paused. The buttons are added on each "TaskbarButtonCreated" (the window shown from the tray, Explorer restarted). Progress updates once per 1% of the track (1–5 s) while playing with the window shown.

**Themed tray menu:** a native popup in the album's ink and glows (or the system colors) with the track and its controls on top and the tray items below; it closes on focus loss, Esc, a click outside or after an item. When Windows refuses it the foreground it holds the mouse capture instead, and then also closes when the capture is lost, the app is deactivated or (a 250 ms timer while open) the foreground window changes, so it is never left open on screen. It is not exposed to screen readers as a menu, which the Windows menu is: switching the plugin off brings that one back.

**Listening stats:** each listen of at least 10 s of played time (counted like the scrobble point) is one JSON line in `stats/plays.jsonl` in the profile; a play is half the track or 30 s, less is a skip. The Stats tab of the settings window asks `settings_stats` for a period (7 days, 30 days, 12 months, all) and gets totals, a daily or monthly series, an hour × weekday map, streaks and the top artists, tracks and albums, computed in Rust from the whole file. Export writes a CSV to Downloads; "clear history" deletes the file. The listen in progress is written on a track change and on quit or restart from the app; a crash or a Windows shutdown loses it.

**Back and forward:** two native buttons left of the app name that call the WebView2's `GoBack` / `GoForward` (no command on the page). `CanGoBack` / `CanGoForward` are read on `HistoryChanged` and only the two buttons are repainted; with nowhere to go a button is dimmed and ignores clicks. The switch is live: turning it off removes the buttons and the name moves back at once. Alt+Left and Alt+Right already work because the browser accelerator keys are left on; the mouse side buttons are not intercepted. In fullscreen the strip is gone, and the buttons with it.

**Song mode:** when a track has both versions, the site shows a Song / Video switch; on each new track (a new video id), if the switch is on Video, the module presses its Song button once. This keeps "the site plays what it serves": it is the choice the user could make with the site's own control, the switch stays visible and no attribute of the site is rewritten, and the site plays whatever it serves for that choice. A click on Video by the user is respected until the next track. It never acts during an ad or before the player exists, and it watches two attributes (`has-av-switcher`, `playback-mode`) instead of polling.

**Precise volume:** the wheel over the player bar or the expanded player moves the volume by 2% (10% with Shift) and calls `preventDefault` only there. The volume goes through the embedded player's public API on `#movie_player` (`getVolume`, `setVolume`, `isMuted`, `unMute`), never through `video.volume`, and the site's slider is set to the same value so the site follows; without that API, or during an ad, nothing happens and the wheel keeps its default. A small "NN%" beside the slider fades out after about 1 s (a CSS transition and one timer that is re-armed, not a loop); it uses the album color variables when `album_color` is on. The optional global keys `[shortcuts] volume_up` / `volume_down` (empty by default) go through `controls`, which asks this module for a 2% step, or takes a 5% step through the same API when it is off.

**Skip disliked:** a track that starts already marked "dislike" (`like-status="DISLIKE"` on the like renderer, read about 1 s after the video id changes) is skipped with the player's next button. A dislike pressed during a track is not acted on (the site handles it). Never during an ad, one skip per video id, and at most five in a row until a track has played for 10 s, so a queue of disliked tracks does not loop.

**Launcher:** on an interactive start (not `--hidden`, `--mini-player` or `--measure`) a small native start screen (Direct2D) opens first and says what the app is doing. The main window is created hidden, so the site loads meanwhile; the window is shown when the update check is done and the site has drawn its first content (a probe on the page, at most 8 s after `load`, 20 s overall), never on a blank page. The update check is one `GET` of the repository's latest release on the GitHub API. A newer release's installer is downloaded, its SHA-256 compared with the one GitHub lists for the asset (the installer is not signed, so a missing or different hash stops the update), and run silently (`/S /R /UPDATE`: it closes the app, replaces it and starts it again). Only an installed copy updates itself (the exe is in the profile folder, where the installer puts it); a build from `target/` never does. Any failure, offline included, just starts the current version.

## Audio

The page's own `<video>` may be processed with Web Audio, locally and for the listener only (see Ground rules). All audio plugins share one page module, `audio`, which is loaded when any of them is on.

- **One graph, created once:** on the first `play`, `createMediaElementSource(video)` takes the element's sound into one `AudioContext`. That capture cannot be undone: from then on the element sounds only through the graph, and the site reuses the same element for every track. So the graph is made once per element, the path to `destination` is never disconnected, and every bypass happens inside the graph.
- **Ads:** while one plays (`#movie_player` has `ad-showing` or `ad-interrupting`, or the site's state says so; no player at all counts as an ad), every parameter goes back to neutral in a 30 ms ramp, nothing is scheduled, measured or skipped, and no other sound plays over it.

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
- Lyrics: how often each source finds a track (and whether a match is ever wrong), the timing quality of lrcmux's KuGou word times, the switch from the first lines to word-timed ones mid-song; the estimated word times on Portuguese and other non-English songs; the Lyrics tab after Next with the player page expanded on an album; the settings window's Lyrics tab, source order and search.
- A same-day baseline to tell whether the native shell costs memory and CPU, and the `animated_title_bar` plugin measured on and off.
- The lyrics cross-fade and in-time scroll in the running app (smoothness, manual scroll and seeks) and their CPU cost; the new parked scroll's timings.
- In the running app: the taskbar buttons and progress (also after Explorer restarts and when running elevated); the themed tray menu (position on each taskbar edge, closing on a click outside, keyboard, 150% scaling); the floating lyrics (readability, word timing against the site, dragging, a second monitor); the Stats tab with a real history and the CSV export.
- The share lyrics window in the running app (the looks on light and dark covers, pasting into chat apps, Save, 150% scaling) and its title bar button; the mini player's lyrics band (the lyrics glyph, timing, the roll between lines, resizing near the screen edge) and its larger buttons with hover labels; the tray menu no longer left open after a click elsewhere.
- The player's fullscreen button (the window goes fullscreen with the site covering it, Esc and the page's exit button restore it, hiding to the tray while fullscreen), the site's bounds after restoring from the tray or moving between monitors of different DPI, and the tray's "Reload page".
- The cost of the five new plugins.
- Back and forward, song mode, precise volume and skip disliked in the running app: the page selectors (`ytmusic-av-toggle .song-button` / `.video-button`, its `aria-pressed`, `has-av-switcher` and `playback-mode`; `#movie_player` `.ad-showing` and its `getVolume` / `setVolume` / `isMuted` / `unMute` / `getVideoData`; `#volume-slider` taking a value; `#main-panel` for the wheel; `#like-button-renderer[like-status]`); the Song / Video switch pressing itself once per track (and not again after a Video click); the volume HUD placement and theme; the wheel step on a mouse and a touchpad; the global volume keys; a disliked track being skipped once (and a queue of them stopping at five); the back and forward buttons dimming and moving the history (and Alt+Left / Alt+Right); the costs of the four plugins.

## Ideas

| Idea | Type |
|---|---|
| More looks (fonts, compact mode, large artwork) | CSS |

## References

- WebView2: [process model](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html)
- Embedded sign-in in WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
- Last.fm [desktop auth](https://www.last.fm/api/desktopauth); [LRCLIB API](https://lrclib.net/docs); [Better Lyrics API](https://docs.betterlyrics.org); [Unison](https://docs.betterlyrics.org/unison); [lrcmux](https://lrcmux.dev)
