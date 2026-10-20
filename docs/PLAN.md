# feathertune: plan

*Português: [PLAN.pt-BR.md](PLAN.pt-BR.md). Keep both files in sync.*

A Rust app (Tauri 2) that opens the music service's **official website** in a lean WebView2 window and handles everything Windows-side in Rust: tray, media keys, integrations and memory. It uses no private API and no official data API, so it does not break when the service changes its protocol, and sign-in and the user's subscription work exactly as in a browser.

## Why this approach

| Approach | Weight | Breaks? | Problem |
|---|---|---|---|
| Native client on private APIs | ~100 MB | yes, all the time | Constant race against protocol and anti-abuse changes, and against the terms of service |
| Official data API + embedded player | ~Chromium | no | Tiny quotas, and its policies forbid background or audio-only playback |
| **Official website in a lean WebView2 (this plan)** | ~450 MB | no (it is the site itself) | Still Chromium inside |

Ground rules that follow from this, and that every phase keeps:
- The page is the official site, unmodified in what it plays. No private APIs, no stream extraction, no downloading, no DRM handling.
- **Ads are never blocked, hidden or altered.** This is permanently out of scope.
- No user-agent spoofing and no automation flags (they also trigger sign-in blocks).
- The app never stores or types credentials; sign-in happens on the site.

## Measured weight

Criterion (same everywhere): Win32_Process, sum of the process tree; private = PrivatePageCount, WS = WorkingSetSize; CPU as % of one core over a 20 s window. Reference: an Electron-based desktop client for the same site, measured on the same machine.

Prototype (`spike/`: bare wry 0.57 + WebView2 155, no optimization, signed out, 1280×800 window, 2026-10-19):

| Scenario | Processes | CPU | Private | Working set |
|---|---|---|---|---|
| Electron client, playing (reference) | 6 | 16–32% | 672 MB | 1009 MB |
| Empty WebView2 (about:blank) | 8 | 0.3% | 121 MB | 319 MB |
| Site idle on home | 9 | 1.5% | 380 MB | 638 MB |
| **Site playing** | 10 | 2.7% | 452 MB | 718 MB |
| **Playing, in the tray** (hidden + memory Low) | 10 | 2.9% | 473 MB | **242 MB** |
| Playing without GPU (`--disable-gpu`) | 10 | 15% | 317 MB | 644 MB |
| Playing with V8 lite (`--js-flags=--lite-mode`) | 10 | 4.5% | 424 MB | 695 MB |

- **Private** is memory only this app holds; **working set** is what is in RAM now, shared memory included.
- In the tray, WebView2 pages the renderer out: private stays, physical RAM drops from 718 to 242 MB, and playback continues.
- No-GPU and V8 lite trade RAM for CPU. **Both are out.**
- Where the RAM goes (playing, private): main process 40 MB + 3.5 MB for our Rust exe, GPU 188 MB, renderer 183 MB, network/audio/storage 33 MB. The gain over Electron comes from dropping Node (−110 MB) and in-page JS plugins. What remains is Chromium (~120 MB fixed) plus the page itself, which no wrapper can cut.

### Targets

| | Electron client | feathertune |
|---|---|---|
| RAM playing, window open | ~670 MB private / ~1 GB WS | **≤ 480 MB private / ≤ 750 MB WS** |
| RAM in the tray | same as open | **≤ 260 MB WS** |
| CPU playing | 16–32% of one core | **≤ 4%** |
| Installer | ~100 MB (bundled Chromium) | **a few MB** (WebView2 ships with Windows) |
| Startup | 4.5 s to content | **2.0 s to the player bar** (Phase 1, warm cache) |

### App measurements

`measure.ps1` at the repo root, signed in, 1280×800 window, one fixed track. Output goes to `results/` (gitignored: it has local paths and listening history).

| Phase | Date | Scenario | Processes | CPU | Private | WS | Startup to bar |
|---|---|---|---|---|---|---|---|
| 1 | 2026-10-19 | Playing, window open | 9 | **2.8%** | **408 MB** | 697 MB | 2.0 s |
| 1 | 2026-10-19 | Playing, in the tray (closed at 25 s, memory Low) | 9 | 2.4% | 418 MB | **167 MB** | 1.9 s |
| 1 | 2026-10-19 | First run, new profile (cold cache) | 9 | — | — | — | 6.4 s |
| 1 | — | 1 h in the tray | pending | | | | |
| 2 | 2026-10-19 | Playing, window open (3 runs, median) | 9 | **3.6%** (3.1–3.9%) | **408 MB** | 711 MB | 2.0 s |
| 2 | 2026-10-19 | Playing, bridge sending 1×/s (discarded; 3 runs, median) | 9 | 6.2% (3.9–6.5%) | 412 MB | 720 MB | 2.0 s |
| 3 | pending | Playing, window open, all integrations on (3 runs, median) | | | | | |
| 3 | pending | Playing, in the tray, all integrations on (3 runs, median) | | | | | |

- Phase 1/2 numbers were taken with the old release profile (thin LTO, 1 codegen unit). The exe process is ~4 MB private either way, so the profile does not move these numbers; the final Phase 4 table uses `--profile dist`.
- In the tray the `<video>` kept advancing and the session stayed signed in.
- Phase 2: every bridge call is a fetch with a CORS preflight. Sending the position 1×/s cost ~3% CPU (A/B on the same build: 3.2% without, 6.2% with). So the page sends only on track change, play/pause and seek, plus a correction every 15 s, and Rust extrapolates the position (`State::position_now()`).

## Architecture

```
feathertune.exe (Rust, Tauri 2)                      WebView2 (the system Chromium)
├─ tray: play/pause/next/quit                        └─ the official site, untouched
├─ close = go to the tray (memory Low)                   ├─ injected script: reads the player state
├─ single instance, autostart, global shortcuts          │   (navigator.mediaSession + <video>)
├─ player state  ◄──── one-way bridge ───────────────────┘   and sends only that to Rust
├─ media keys / Windows media flyout (SMTC)              injected CSS: cuts animation and blur (Phase 4)
├─ Discord Rich Presence, Last.fm, track notification
└─ tracker filter (privacy lists only, Phase 4)
```

- **Rust:** everything Windows (tray, SMTC, shortcuts, autostart, notifications), the integrations, memory control (hidden → `MemoryUsageTargetLevel=Low`, shown → `Normal`) and the network filter.
- **Page:** the site as delivered. Sign-in, library, lyrics, radio and recommendations come with it; the app reimplements none of it.
- **Bridge:** an injected script (`initialization_script`) observes the player and sends Rust a JSON with title, artist, artwork, time and state. Rust treats it as untrusted data: validates it and executes nothing. Rust → page commands (pause, next) click the player buttons; if the site changes its UI, about 3 CSS selectors break, not a protocol.
- **Security:** the remote page gets no Tauri command except the state report; navigation is pinned to the site and sign-in domains (everything else opens in the default browser).

## Risks

1. **Sign-in inside WebView2:** works (2026-10-19). Providers block embedded sign-in mainly on spoofed user agents or automation flags, so the app must never use either.
2. **Shared runtime:** WebView2 is updated by Windows. Good for security and codecs, but an update can change resource use with no change on our side.
3. **Terms of service:** wrapping the website the user signs into is what a browser does; no API policy applies since no API is used. Blocking or altering ads would cross that line, so it is never done.

## Phases

**How to run:** open a new session with the listed model and ask "execute Phase N of `docs/PLAN.md`" (prompts in [PROMPTS.md](PROMPTS.md)). One phase per session, in order. Every phase ends with a measurement (`measure.ps1`, run by the maintainer on Windows) and the result written to the table above, in both plan files.

**Language:** all code in English (identifiers, comments, file names, `config.toml` keys, logs, commit messages). Docs exist in English and Brazilian Portuguese and are kept in sync.

**Model split:** Opus 5.5 (medium) for COM/WinRT interop with WebView2 and Windows, remote-page security and the architecture later phases inherit. Sonnet 5.5 (high) for well-specified work: official Tauri plugins, public HTTP APIs and measurement-driven experiments.

| Phase | Model | Why |
|---|---|---|
| 1. Shell | Opus 5.5 medium | structure, remote-page capabilities, memory COM interop, the shared meter |
| 2. Bridge and media keys | Opus 5.5 medium | untrusted page → Rust channel, SMTC via WinRT |
| 3. Integrations | Sonnet 5.5 high | official plugins and public APIs on top of a ready state |
| 4. Slimming and delivery | Sonnet 5.5 high | measured experiments, installer |

### Phase 1: Shell (done 2026-10-19)

Tauri 2.12 app in `app/`, no frontend of its own: the main window opens the site. WebView2 profile in `%LOCALAPPDATA%\feathertune`. Chromium flags in a single `--disable-features` (Chromium only honors the last one). Navigation pinned to the site and sign-in domains. Closing hides to the tray with `MemoryUsageTargetLevel=Low`. Tray: show/hide, play/pause, next, quit.

**Injections as modules** (seed of future plugins): every JS/CSS that enters the page lives in `app/injections/<id>/` (`page.js` and/or `style.css`), has an on/off switch under `[injections]` in the profile's `config.toml`, and is assembled into a single `initialization_script`, each module isolated in its own `try/catch`.

Result: playing 408 MB private and 2.8% CPU, tray 167 MB WS, startup to the player bar 2.0 s, sign-in persists across restarts. **Pending:** the 1 h tray test (`./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60`, without opening the app during the hour; done when final WS ≤ WS at 5 min + 10%).

### Phase 2: Bridge and media keys (done 2026-10-20)

- `core` injection (always on, main frame only) → single command `player_state` → `Player` (`app/src/player.rs`, API documented at the top). Capability `app/capabilities/player-bridge.json`: only that command, only window `main`, only the site's origin, nothing for local origins. `frontendDist` was removed from `tauri.conf.json`: with it, Tauri treated the remote site as a *local* origin, and any future local capability would leak to the page.
- WebView2's own SMTC entry showed as `msedgewebview2.exe`, so it is turned off (`HardwareMediaKeyHandling`) and the app publishes its own (`app/src/smtc.rs`, tied to the main window).
- **Flyout name:** a bare exe shows as "Unknown app". The fix is a process AppUserModelID (`dev.feathertune.desktop`) plus a Start menu shortcut carrying the same ID. The app creates `%APPDATA%\Microsoft\Windows\Start Menu\Programs\feathertune.lnk` if missing; the Phase 4 installer creates it with the same AUMID and the uninstaller removes it (see Phase 4).
- Media keys with the app in the tray: play/pause ≤ 16 ms, next ~0.5 s, previous ~0.2 s until the new state reaches Rust. CPU playing: median 3.6%, 408 MB private.

### Phase 3: Integrations (Sonnet 5.5 high) — code written 2026-10-20, maintainer checks and measurement pending

Consumes only the Phase 2 state. Does not touch the bridge, the WebView or the flags.

- Official plugins: `single-instance` (a second launch shows the existing window), `autostart` (tray menu option, off by default), `window-state`.
- Track-change notification, only while the window is hidden.
- Global shortcuts (`tauri-plugin-global-shortcut`): play/pause, next, previous. Keys in the profile's `config.toml`.
- Discord Rich Presence (`discord-rich-presence` crate): title, artist and time; cleared on pause; silent failure and reconnect if Discord is closed.
- Last.fm:
  - the user's own API key, read from local config, never in code or Git;
  - desktop auth (`auth.getToken` → browser → `auth.getSession`), session key in the Windows Credential Manager (`keyring` crate);
  - `track.updateNowPlaying` on track start; `track.scrobble` when the track is longer than 30 s and half of it or 4 min has played, whichever comes first;
  - simple offline queue for failed scrobbles.
- **Done when:** each item works with the app in the tray, the maintainer confirms Discord and the scrobble on a real Last.fm profile, and CPU playing stays ≤ 4%.

Implementation notes (`app/src/{settings,shortcuts,notify,discord,lastfm}.rs`; each integration is one thread blocked on `Player::subscribe()`, with no polling):

- All settings come from the profile's `config.toml` (`[shortcuts]`, `[notifications]`, `[discord]`, `[lastfm]`; see the README). The Discord application id and the Last.fm key and secret are the maintainer's own and live only there, outside Git. An empty value turns the integration off.
- **Notification without the plugin:** `tauri-plugin-notification` injects a script into every webview that replaces `window.Notification` in the page, which breaks "the page is untrusted and we do not touch it". The toast is shown from Rust with `tauri-winrt-notification` (the crate the plugin uses), under the app's AppUserModelID.
- **Autostart** passes `--hidden`, so a login start goes straight to the tray (window created invisible, memory Low). **Window state** saves size, position and maximized only (never visibility) and is disabled with `--measure`, so measurements keep the fixed 1280×800.
- **Shortcut defaults:** `Ctrl+Alt+Space`, `Ctrl+Alt+Right`, `Ctrl+Alt+Left`; `""` turns one off; a combination another app owns is logged and skipped.
- **Discord:** "Listening" activity (title, artist, artwork, time bar from the extrapolated position); sent on track change and resume, and again only if the position drifted > 2 s (a seek); cleared on pause. If Discord is closed the connection is retried at the next player event.
- **Last.fm:** the tray item connects or disconnects. Sign-in opens the browser and polls `auth.getSession` every 5 s for 5 min; the session key goes to the Credential Manager (service `feathertune`). The scrobble point counts played time only (pauses excluded) and a timer wakes the thread at that moment. Replaying the same track (position goes back after the scrobble) counts as a new listen. Failed scrobbles wait in `lastfm-queue.json` in the profile (batches of 10, retried every 5 min, at startup and at the next scrobble; entries older than 14 days are dropped because Last.fm ignores them). A rejected session key (error 9) drops the session and asks for a new sign-in.
- **Cloud session limits:** `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` and `cargo fmt --check` pass from Linux; the app was not run. Pending for the maintainer, on Windows: each item in the tray, the Discord check, a real scrobble, and `measure.ps1` (3 runs, median) for the two rows above. When all of it passes, replace the heading note with "Phase 3: done on <date>" and fill the table.

### Phase 4: Slimming and delivery (Sonnet 5.5 high) — code written 2026-10-20, maintainer measurements and install check pending

Everything below is implemented; nothing in it has been run. The cloud session cannot start the app, run `measure.ps1` or install, so every measured cell is `pending` and the phase is **not** marked done. Heading note to replace with "Phase 4: done on <date>" once the tables are filled and the install checklist passes.

**Experiments** (one at a time, each measured with `measure.ps1`, 3 runs, median; only what shows a gain stays). Every switch is in the profile's `config.toml`, so the whole table needs **one** build:

```toml
[injections]
performance = true                 # CSS: no animation, transition, smooth scroll or backdrop-filter
[browser]
disable_features = ["Name1", "Name2"]   # added to the single --disable-features (names: letters, digits, _)
[privacy]
tracker_filter = true              # the tracker filter below
```

```powershell
cd app; cargo build --release; cd ..                     # once for the whole phase
$a = '--measure', '<track url>'
1..3 | % { ./measure.ps1 -Name base-open-$_ -ArgumentList $a }                         # window open
./median.ps1 -Prefix base-open-
1..3 | % { ./measure.ps1 -Name base-tray-$_ -ArgumentList $a -HideAfter 25 -Wait 50 }  # in the tray
./median.ps1 -Prefix base-tray-
```

Repeat with a new prefix (`e1-open-`, `e1-tray-`, ...) after changing only that experiment's line in `config.toml`, and put the two `median.ps1` outputs in the table. **Rule:** a change stays only if its median beats the baseline by more than the baseline's own min–max spread (Phase 2: CPU 3.1–3.9%) *and* sign-in and playback still work. A feature name Chromium does not know is ignored without an error, so "no change" can also mean the name does not exist in the installed runtime. After the single experiments, measure the kept ones together once.

| # | Experiment (switch) | Open: CPU / private / WS | Tray: WS | Decision |
|---|---|---|---|---|
| E0 | Baseline: current app, everything below off | pending | pending | — |
| E1 | CSS `performance` module (`performance = true`) | pending | pending | pending |
| E2 | `SpareRendererForSitePerProcess`: no pre-created spare renderer process | pending | pending | pending |
| E3 | `Prerender2`: no page prerendering | pending | pending | pending |
| E4 | `MediaRouter`, `DialMediaRouteProvider`: no Cast discovery | pending | pending | pending |
| E5 | `OptimizationHints`, `OptimizationGuideModelDownloading`, `OptimizationHintsFetching`, `OptimizationTargetPrediction`: no hints/model downloads | pending | pending | pending |
| E6 | Tracker filter, bundled list (`tracker_filter = true`) | pending | pending | pending |
| E7 | Tracker filter + a privacy list in `privacy-list.txt` (for example EasyPrivacy) | pending | pending | pending |
| E8 | Everything that stayed, together | pending | pending | — |

Considered and **not** tried, on purpose: site isolation off (`site-per-process`, weakens isolation of an untrusted page); `--disable-component-update` (could freeze the runtime's media components and break playback); Privacy Sandbox / ad-related features (touch ads); `--disable-gpu` and V8 lite mode (already out, see Measured weight); `CalculateNativeWinOcclusion` (the tray already hides the view explicitly, and this one would make hidden windows count as visible). A feature that stays moves from `disable_features` into `DISABLED_FEATURES` in `app/src/main.rs`; a module or experiment that does not is deleted, and its row here keeps the numbers.

**`performance` module** (`app/injections/performance/style.css`): sets animation/transition durations and delays to `0s` (not `animation: none`, so `animationend`/`transitionend` handlers still fire), `scroll-behavior: auto`, `backdrop-filter: none`. Presentation only. Off by default until E1 shows a gain. Loading spinners stop by design; anything else that looks broken in E1 (or an element living in a native shadow root, which a page-level selector does not reach) is a reason to drop the module.

**Tracker filter** (`app/src/privacy.rs`, `[privacy] tracker_filter`, off by default): WebView2 `WebResourceRequested` through `with_webview` + the `adblock` crate (0.13, default features) with cosmetic rules dropped, so it can only block requests, never change the page. Built to be unable to touch ads or playback, in code and not by trusting the list:

- the handler is registered only for scripts, images, XHR/fetch, pings and "other"; documents, frames, media, fonts and stylesheets never reach it;
- hosts on the player's domains, any Google domain and Google's ad-delivery domains (doubleclick, googlesyndication, googleadservices, 2mdn, ...) are skipped before the list is asked, so even an ad list dropped in by mistake changes nothing there;
- the bundled list (`app/filters/privacy.txt`, written for this project) has only third-party analytics and session-recording hosts. More rules go in `privacy-list.txt` in the profile (privacy lists only: the maintainer's call, and E7 measures the cost of a full one);
- a blocked request gets an empty 403. Under `--measure` each blocked host is printed once (`tracker_blocked=<host>`), so a run shows what was caught. If nothing is ever blocked on the site, the filter has no gain and is a candidate for removal, whatever it costs per request.
- Status: the COM code compiles and the list logic has unit tests; **the filter was not run**. Its handler is called on the UI thread for every subresource of those kinds, so E6/E7 measure that cost too. It did not stall.

**Installer** (`app/tauri.conf.json` → `bundle`, `app/installer-hooks.nsh`): Tauri's NSIS bundler, per-user (no admin; installs to `%LOCALAPPDATA%\Programs\feathertune`), WebView2 `downloadBootstrapper` (adds nothing to the installer; fetches the bootstrapper only where the runtime is missing, which Windows 11 and an updated Windows 10 are not; `embedBootstrapper` would add ~1.8 MB for offline installs), English and Brazilian Portuguese by system language. CI builds it with `build -- --profile dist --locked` (the `dist` job; artifact `feathertune-<sha>`, installer at `bundle/nsis/*-setup.exe`). By hand: `cd app; npx @tauri-apps/cli@2.12.3 build -- --profile dist`.

- **Shortcut and AUMID:** the stock template creates `Start Menu\Programs\feathertune.lnk` and stamps it with the bundle identifier as AppUserModelID, which is `dev.feathertune.desktop`, the ID `smtc.rs` sets on the process. It is the same path the app writes when the shortcut is missing, so there is no duplicate, and the uninstaller removes it (when it still points at the installed exe), plus the jump list and the "Start with Windows" entry.
- **Profile:** the template's "delete application data" checkbox clears folders named after the bundle identifier, not `%LOCALAPPDATA%\feathertune`, so `installer-hooks.nsh` clears the real profile (sign-in, `config.toml`, Last.fm queue) and the Last.fm credential (`cmdkey /delete:lastfm-session.feathertune`, target name derived from the `keyring` crate source, **to be confirmed**) only when the box is ticked. Unticked, nothing of the user's is removed.
- **Install check for the maintainer** ("installs and uninstalls cleanly"), on Windows:
  1. install, start from the Start menu: window opens, signed-in profile or sign-in works, the media flyout shows the app name, the shortcut has the AUMID (`dev.feathertune.desktop`);
  2. install again over the running app: it asks to close it, the profile survives;
  3. uninstall **unticked**: install folder, shortcut and Run entry gone; `%LOCALAPPDATA%\feathertune` still there; reinstall keeps the sign-in;
  4. uninstall **ticked**: profile folder gone, and `cmdkey /list | findstr lastfm` empty.

**Final table: Electron client × feathertune.** `--profile dist` build, 3 runs, median, same criterion as above. Electron: `./measure.ps1 -Name electron-open-$_ -Exe <client exe> -ArgumentList <its args>` with the same track playing; feathertune: `-Exe app/target/dist/feathertune.exe`. Fill in `median.ps1` output.

| | Electron client | feathertune (dist) | Target |
|---|---|---|---|
| Playing, window open: CPU | pending | pending | ≤ 4% |
| Playing, window open: private | pending | pending | ≤ 480 MB |
| Playing, window open: WS | pending | pending | ≤ 750 MB |
| Playing, in the tray: WS | pending | pending | ≤ 260 MB |
| Processes | pending | pending | — |
| Startup to the player bar / content | pending | pending | ≤ 2.0 s |
| Installer size | pending | pending | a few MB |

**Cloud session limits:** `cargo clippy --target x86_64-pc-windows-msvc --all-targets -- -D warnings` and `cargo fmt --check` run from Linux; the CI `dist` job builds the installer on Windows. Nothing was run. Pending for the maintainer: the baseline and E1–E8, the final table, the install checklist, the Phase 3 checks that were already pending, and filling the Phase 3 and 4 heading notes.

## After Phase 4: plugins (future, out of current scope)

Goal: plugins you switch on and off. An injection module (`app/injections/<id>/`), a Rust part compiled into the app implementing a `Plugin` trait (receives the state, adds menu items), or both. On/off in a "Plugins" tray menu, saved in `config.toml`. No DLL loading and no code downloaded at runtime. Each plugin is measured on and off and its cost goes in its description; all of them on must stay within the CPU and RAM targets.

| Candidate | Type |
|---|---|
| Themes and look (colors, font, compact mode, large artwork) | CSS |
| Performance (the Phase 4 module, with levels) | CSS |
| Audio equalizer and compressor (Web Audio on the `<video>`) | JS |
| Media buttons on the taskbar thumbnail | Rust (Win32) |

Before the first plugin, an Opus 5.5 medium session formalizes the `Plugin` trait and the menu.

## Reproducing the measurements

`measure.ps1` measures any exe (default: the app's release build). Close the app first: two instances on the same profile share the WebView2 browser process and the tree comes out wrong. `--measure` makes the app print the time to the player bar (`player_bar_ms`) and, every 10 s, the `<video>` time and sign-in state.

```powershell
cd app; cargo build --release; cd ..
$a = '--measure', '<track url>'
./measure.ps1 -Name playing -ArgumentList $a
./measure.ps1 -Name tray -ArgumentList $a -HideAfter 25 -Wait 50
./measure.ps1 -Name tray-1h -ArgumentList $a -HideAfter 25 -Minutes 60
1..3 | % { ./measure.ps1 -Name final-$_ -Exe app/target/dist/feathertune.exe -ArgumentList $a }   # Phase 4 table
./median.ps1 -Prefix final-                                                                       # median of the 3 runs
./measure.ps1 -Name spike -Exe spike/target/release/spike.exe -ArgumentList '"<url>"', 'spike/data/x'
```

`median.ps1 -Prefix <p>` prints the median and the min–max range of every `results/<p>*.json`, for the 3-run rows of the tables.

`-HideAfter` sends WM_CLOSE to the window (the app goes to the tray). The app uses the real profile, so it measures signed in.

## References

- WebView2: [process model](https://learn.microsoft.com/microsoft-edge/webview2/concepts/process-model), [performance](https://learn.microsoft.com/microsoft-edge/webview2/concepts/performance), [MemoryUsageTargetLevel](https://learn.microsoft.com/dotnet/api/microsoft.web.webview2.core.corewebview2.memoryusagetargetlevel)
- Tauri 2: [WebviewWindowBuilder](https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html) (`initialization_script`, `additional_browser_args`)
- Embedded sign-in in WebView2: [WebView2Feedback #1584](https://github.com/MicrosoftEdge/WebView2Feedback/issues/1584)
