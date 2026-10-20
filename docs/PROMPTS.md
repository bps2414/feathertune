# Phase prompts

*Português: [PROMPTS.pt-BR.md](PROMPTS.pt-BR.md).*

One new session per phase, in order, with the listed model. Copy the whole block. Phases 1 and 2 are done. The repo rules (build discipline, legal limits, language) are in [AGENTS.md](../AGENTS.md); every session reads it.

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
Out of scope: the plugins in "After Phase 4".

Rules:
- Measurements run on Windows with measure.ps1. In a cloud session, implement and let CI build; I run the measurements.

Done when: the final Electron client × feathertune table is in both plan files (3 runs, same criterion), the installer installs and uninstalls cleanly (the profile is removed only if the user asks), and both plan files say "Phase 4: done on <date>".

At the end, one commit with the whole phase: "Phase 4: slimming and installer".
```
