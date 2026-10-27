# AGENTS.md

Instructions for coding agents working in this repo. Humans: see [README.md](README.md) and, to send a pull request, [CONTRIBUTING.md](CONTRIBUTING.md) (pt-BR: [CONTRIBUTING.pt-BR.md](CONTRIBUTING.pt-BR.md)). A pull request written with an agent follows the same rules as one written by hand; see "Pull requests" below.

feathertune is a Windows-only Tauri 2 app that hosts a music service's official website in a lean WebView2 window. Design, security model, measurements, what is not verified yet and ideas are in [docs/DESIGN.md](docs/DESIGN.md) (pt-BR: [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md)).

## Layout

- `app/`: the app (Cargo project; run cargo commands from here). Each `src/*.rs` starts with a `//!` comment that says what it is; read it before editing the module. Tests live inside the modules (`#[cfg(test)]`).
  - Shell and native UI (Direct2D): `main.rs` window, tray, navigation; `titlebar.rs`, `miniplayer.rs`, `splash.rs` (start screen), `traymenu.rs`, `taskbar.rs`, `gfx.rs` (shared drawing); `settings.rs` (`config.toml`), `settings_window.rs` + `settings/` (the settings window); `plugins.rs` the plugin list with costs; `i18n.rs` + `locales/pt-BR.json` interface texts (`t("English")`); `game_mode.rs`; `shortcuts.rs`; `notify.rs`; `updater.rs` self-update; `diag.rs` the diagnostics export.
  - Player and integrations: `player.rs` player state (what the integrations consume); `smtc.rs` media flyout; `discord.rs`; `lastfm.rs`; `stats.rs` listening stats.
  - Lyrics and art: `lyrics.rs` sources, cache and translation; `lyric_view.rs` current/next line (floating lyrics, mini player); `desktop_lyrics.rs`; `share_lyrics.rs` picture cards; `artwork.rs` + `album_color.rs` cover and album colors.
  - Audio: `audio.rs` + `injections/audio/` the audio graph (equalizer, compressor, crossfade, analysis, skip silence, visualizer, mix buttons); `audio_facts.rs` track analysis store; `mixes.rs` mixes and the Mix tab's command; `deck.rs` + `injections/deck/` the transition voice.
  - Page: `injections.rs` + `injections/<id>/` page scripts (`core`, `controls`, `theme`, `lyrics`, `ambient`, `audio`, `deck`, `precise_volume`, `skip_disliked`, `song_mode`); `privacy.rs` + `filters/` tracker filter; `capabilities/` + `permissions/` what the remote page may call.
  - Packaging: `tauri.conf.json` + `installer-hooks.nsh` the NSIS installer.
- `spike/`: frozen measurement prototype. Do not build it unless a task is about it.
- `measure.ps1`: process-tree meter (Windows, needs a signed-in profile; run by the maintainer). `median.ps1`: median of several of its runs. `measure-plugins.ps1`: runs `measure.ps1` for every plugin (about 2 h; maintainer only).
- `docs/`: `DESIGN.md` and `REMOTE.md` (plan for the phone remote, which will live in `mobile/`), in English and pt-BR.
- `.github/`: `workflows/ci.yml`, the pull request template and the issue forms. `LICENSE` (MIT), `SECURITY.md`, `CONTRIBUTING.md` (+ pt-BR).
- Local only, gitignored: `results/` (measure output) and `plans/` (work plans, pt-BR). Do not commit or quote them.

## Build discipline (important: builds are slow, do not repeat them)

Timings on the maintainer's machine after a one-file change: `cargo check` ~1 s, `cargo build` ~2 s, `cargo build --release` ~9 s, `cargo clippy` ~4 s. A cold build is ~1–2 min per profile, and `--profile dist` (thin LTO, 1 codegen unit) is several minutes.

- Iterate with `cargo check`. It is the only command you need while editing.
- Run `cargo clippy --all-targets -- -D warnings` and `cargo fmt` once before committing, not after each edit.
- Build only when you are about to *run* the exe, and build once per batch of changes, not per edit. If nothing under `app/` changed since the last build, do not rebuild.
- Use the default `release` profile for manual runs and measurements. `--profile dist` is only for the installer and release measurements.
- Never `cargo clean`, never delete `target/`, never disable caches or incremental builds. Do not add `CARGO_INCREMENTAL=0` locally (CI sets it on its own).
- Do not chain "build + run a test script" in a loop. Read the code and the error first, then build once.
- Run long commands (release builds, `measure.ps1`) in the background and wait for them; do not start a second build while one runs (they block on the same `target/` lock).
- Pipe long cargo output through `tail` / `Select-Object -Last` and show only errors.
- Do not add dependencies or Cargo features casually: every crate adds cold-build time. Prefer what is already in `Cargo.toml`.

Build settings that make this fast (keep them): `.cargo/config.toml` links with `rust-lld`; `Cargo.toml` has no debug info for dependencies, an incremental non-LTO `release`, and a separate `dist` profile.

## Cloud sessions (Linux) and CI

The app targets Windows only (`windows` crate, WebView2), so it does not build or run on Linux. In a cloud session:
- Edit, then push the branch and let the GitHub Actions workflow (`.github/workflows/ci.yml`, Windows runner) run fmt, clippy, tests and a build. Read its result with `gh run list` / `gh run view --log-failed` instead of guessing.
- `cargo check --target x86_64-pc-windows-msvc` from Linux is not verified to work (`tauri-build` compiles Windows resources); try it at most once, and fall back to CI.
- Pull requests: see "Pull requests" below. Do not bump versions, edit the changelogs or push tags in one.
- Releases: bump `version` in `app/Cargo.toml` and `app/tauri.conf.json` together, add the version's section to `CHANGELOG.md` and `CHANGELOG.pt-BR.md` (the release notes come from the English one; CI fails without it), then push a `vX.Y.Z` tag; the CI `dist` job publishes the installer and the `android` job attaches the phone APK (bump `version` in `mobile/app.json` and `mobile/package.json` and `android.versionCode` in `mobile/app.json` when the phone app changed). Only when the maintainer asks.
- Anything that needs the running app, a signed-in account or `measure.ps1` is left for the maintainer: say so in the summary instead of claiming it works.

## Pull requests

[CONTRIBUTING.md](CONTRIBUTING.md) is the full guide; these are the parts an agent must not miss.

- Check `git status` and `git diff` first: the diff must hold only what the task asked for.
- One subject per pull request; title and commits in English, imperative, starting with the area (`Lyrics: keep the line lit until its end`). Branch from `main`, rebased on it.
- Run, once and in this order, from `app/`: `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`. This is what CI runs on the PR (fmt, clippy, tests only: it does not open the app).
- Fill `.github/PULL_REQUEST_TEMPLATE.md`. Say what was tested and what was not; never claim a behavior that needs the running app, a signed-in account or `measure.ps1` works.
- New interface text goes through `t("...")` with an entry in `app/locales/pt-BR.json`. A changed doc changes its pair.
- Do not change `version`, the changelogs or tags; the maintainer writes release notes.
- Say in the PR when an agent wrote part of it. A change that needs a decision (new dependency, plugin, wider `capabilities/`, anything in "Legal and safety limits") goes to an issue first, not straight to a PR.
- Never put personal data in a pull request: local paths, usernames, emails, tokens, account names, listening history, screenshots or diagnostics with any of them.

## Code rules

- All code in English: identifiers, comments, file names, `config.toml` keys, logs, commit messages.
- Docs exist in English and pt-BR. When you change one, change its pair (`README.md`/`README.pt-BR.md`, `docs/DESIGN.md`/`docs/DESIGN.pt-BR.md`).
- Match the surrounding style (`rustfmt.toml`: width 120). Minimal changes, no speculative abstractions.
- Every Chromium feature flag goes in the single `--disable-features` in `BROWSER_ARGS` (Chromium only honors the last one).
- UI work (native or HTML) follows the performance rules: draw only on change; any animation has a plugin switch, runs on one timer only while something is on screen to move (stops when the window is hidden, minimized or in the background) and resumes where it stopped; Windows animation effects and `prefers-reduced-motion` do not turn it off (the switch does; the page gets `--force-prefers-no-reduced-motion`); invalidate only the region that changes; keep GPU resources (brushes, layouts, bitmaps) between frames.
- Every plugin is on by default, except those in the settings' Experimental tab (off until reliable; see docs/DESIGN.md, Plugins). Write its measured cost in `plugins.rs` and docs/DESIGN.md (or "not measured yet" until the maintainer measures it).
- The page is untrusted. It may only call `player_state`, `audio_facts` (numbers measured from a track) and `mix_open` (opens our Mix tab on a pair, nothing else); their payloads are validated and never trigger an action on the player. Do not widen `capabilities/` or `allowed()` without a reason written in docs/DESIGN.md.

## Legal and safety limits (never cross)

- Never block, hide, skip, shorten, mute, fade or cover ads; every audio effect is bypassed while one plays. Network filtering is privacy lists only.
- The audio the page plays may be processed and measured locally, for the listener only (Web Audio on the page's own media element). Never record, store, upload or re-encode audio and never fetch a stream; only numbers derived from it (tempo, key, a loudness envelope) may be kept, in the profile.
- No private APIs, stream extraction, downloading or DRM handling.
- No user-agent spoofing and no automation flags.
- Never type or store account credentials; sign-in and account checks are done by the maintainer.
- Keep brand and product names out of the repo name, UI text, README and docs. Domains and CSS selectors in code are fine where they are functional.
- No personal data in commits: no local paths, usernames, emails, tokens, API keys or listening history. `measure.ps1` output goes to `results/`, which is gitignored; keep it that way.
