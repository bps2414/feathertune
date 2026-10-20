# Phase prompts

*Português: [PROMPTS.pt-BR.md](PROMPTS.pt-BR.md).*

One new session per phase, in order, with the listed model. Copy the whole block. Phases 1 to 4 are committed (Phases 3 and 4 have maintainer checks open). The repo rules (build discipline, legal limits, language) are in [AGENTS.md](../AGENTS.md); every session reads it.

## Phase 3: Integrations (Sonnet 5.5, effort high)

```
Execute Phase 3 of feathertune, described in docs/PLAN.md ("Phase 3: Integrations").

First read AGENTS.md, docs/PLAN.md and the code in app/. Read the `Player` module carefully (its API is documented at the top of app/src/player.rs): it is the only part of Phase 2 you consume. Run `git log --oneline` to confirm Phases 1 and 2 are committed; if not, stop and tell me.

Scope: Phase 3 only: single-instance, autostart (off by default, tray menu option) and window-state plugins; track-change notification only while the window is hidden; global shortcuts configurable in the profile's config.toml; Discord Rich Presence; Last.fm (now playing + scrobble by the 30 s / half / 4 min rule, with an offline queue). Do not touch the injected scripts, the `player_state` command, the WebView, the flags or the SMTC.

Rules:
- Use the current versions of the official Tauri 2 plugins and check each crate's docs before using it.
- The Last.fm API key is mine: read it from local config outside Git, never from code. The session key goes to the Windows Credential Manager (keyring crate).
- Last.fm auth and the Discord/scrobble confirmations are done by me with my accounts. Never type credentials.
- If you are in a cloud session (Linux), you cannot run the app: rely on the Windows CI (see AGENTS.md), and leave the in-tray checks and the measurement for me.

Done when: each item works with the app in the tray, I confirmed Discord and the scrobble on my Last.fm profile, CPU playing stays ≤ 4% (measure.ps1), and both plan files say "Phase 3: done on <date>" with the measurement.

At the end, one commit with the whole phase: "Phase 3: integrations".
```

## Phase 4: Slimming and delivery (Sonnet 5.5, effort high)

```
Execute Phase 4 of feathertune, described in docs/PLAN.md ("Phase 4: Slimming and delivery").

First read AGENTS.md, docs/PLAN.md, the code in app/ and measure.ps1. Run `git log --oneline` to confirm Phases 1 to 3 are committed; if not, stop and tell me.

Scope: Phase 4 only.
1. Baseline of the current app (3 runs, median): playing with the window open and in the tray.
2. Experiments one at a time, each measured the same way: CSS that removes animations/transitions/backdrop-filter, as a `performance` injection module, and flags for unused features. Only what shows a gain stays; every experiment (kept or dropped) goes in a table in both plan files.
3. Tracker filter: WebView2 WebResourceRequested via with_webview + the adblock crate with a privacy list only. Never an ad list, never anything that changes what the player plays. If this item stalls, stop just this item, document where it stalled in the plan and go on.
4. Tauri NSIS installer with the WebView2 bootstrapper, built with --profile dist. It creates the Start menu shortcut with the app's AUMID and the uninstaller removes it.
Out of scope: the plugins (Phases 5 and 6).

Rules:
- Measurements run on Windows with measure.ps1. In a cloud session, implement and let CI build; I run the measurements.

Done when: the final Electron client × feathertune table is in both plan files (3 runs, same criterion), the installer installs and uninstalls cleanly (the profile is removed only if the user asks), and both plan files say "Phase 4: done on <date>".

At the end, one commit with the whole phase: "Phase 4: slimming and installer".
```

## Phase 5: Native shell (Opus 5.5, effort medium)

```
Execute Phase 5 of feathertune, described in docs/PLAN.md ("Phase 5: Native shell").

First read AGENTS.md, docs/PLAN.md and the code in app/ (main.rs, player.rs, smtc.rs, privacy.rs, settings.rs, lastfm.rs, capabilities/). Run `git log --oneline` to confirm Phases 1 to 4 are committed; if not, stop and tell me.

Scope: Phase 5 only.
1. Native title bar: decorations off, a strip drawn by Rust with Direct2D/DirectWrite (name, menu, minimize, maximize/restore, close), the site as a child webview below it (Tauri multiwebview, `unstable` feature), native WM_NCHITTEST with snap layouts and resize borders, DPI-aware, light/dark and a theme color hook for Phase 6.
2. Settings window: our own HTML/CSS from a custom URI scheme (never frontendDist), its own capability for that window and scheme only, created on open and destroyed on close; writes config.toml keeping comments; lists every plugin with its switch.
3. Plugin model: every optional feature (injection module and/or Rust integration) declared with id, name, description, cost and a config.toml switch, listed by the settings window. No DLLs, no downloaded code.
4. Last.fm API key and secret from the CI secrets FEATHERTUNE_LASTFM_API_KEY / FEATHERTUNE_LASTFM_API_SECRET via option_env! in the dist job; config.toml overrides; builds without them behave as today.
5. Native always-on-top mini player (Direct2D, artwork decoded with WIC, fetched by Rust only from the site's image hosts), fed only by Player; opening it hides the main window with memory Low.
Out of scope: album color, lyrics and ambient mode (Phase 6).

Rules:
- The site's capability does not change: player_state only, site webview only. The settings window's capability must not reach the site webview, and the site must not reach the settings window's commands. Write in the plan how this is enforced.
- Check the current Tauri 2 multiwebview, custom URI scheme and capability docs before using them (ctx7), and the `windows` crate features you add. No new crate except toml_edit if `toml` cannot keep comments.
- Keep everything that already works on the child webview: memory Low when hidden, the tracker filter, SMTC/flyout, single-instance, window-state, `--measure`.
- If multiwebview blocks (focus, keyboard, memory Low or SMTC break on the child webview), stop item 1, use the DWM-colored system frame fallback described in the plan, write why in the plan, and go on.
- I create the two GitHub secrets and confirm the Last.fm sign-in with my account. Never type credentials.
- In a cloud session (Linux) you cannot run the app: rely on the Windows CI and leave the window checks and measurements for me.

Done when: the plan's "Done when" list for Phase 5 is met, measured with measure.ps1 (3 runs, median) and written to the table in both plan files, the README status is updated in both languages, and both plan files say "Phase 5: done on <date>".

At the end, one commit with the whole phase: "Phase 5: native shell".
```

## Phase 6: Look (Sonnet 5.5, effort high)

```
Execute Phase 6 of feathertune, described in docs/PLAN.md ("Phase 6: Look").

First read AGENTS.md, docs/PLAN.md and the code in app/, especially the Phase 5 plugin model, the settings window, the title bar theme color hook and the WIC artwork decoder. Run `git log --oneline` to confirm Phase 5 is committed; if not, stop and tell me.

Scope: Phase 6 only, each item a plugin declared in the Phase 5 model and switchable from the settings window.
1. Album color: dominant color of the current artwork (Phase 5 decoder, once per track) applied to the title bar, the mini player and the page through a CSS variable in app/injections/theme/. Presentation only, readable text contrast.
2. Synced lyrics: Rust fetches https://lrclib.net/api/get (track_name, artist_name, album_name, duration; syncedLyrics) with a User-Agent naming the app, one request per track, cached in the profile, and pushes the timed lines as JSON data to app/injections/lyrics/, which shows them in the site's Lyrics tab following the <video> time. When LRCLIB has nothing, leave the site's own lyrics alone.
3. Ambient mode: blurred, glowing artwork behind the player (injection module), paused while hidden, off by default.

Rules:
- The page stays untrusted: it fetches nothing new and calls nothing in Rust for these; data goes Rust → page only, serialized as JSON, never as code built from strings. Do not widen capabilities/.
- Injection modules stay presentation only and never touch ads or what the player plays.
- Measure each plugin on and off with measure.ps1 (3 runs, median) and apply the Phase 4 rule; the defaults follow the numbers, except ambient mode, which is off.
- In a cloud session (Linux) you cannot run the app: rely on the Windows CI and leave the checks and measurements for me.

Done when: the plan's "Done when" for Phase 6 is met, every measurement is in the table in both plan files, the README status is updated in both languages, and both plan files say "Phase 6: done on <date>".

At the end, one commit with the whole phase: "Phase 6: look".
```
