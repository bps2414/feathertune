# AGENTS.md

Instructions for coding agents working in this repo. Humans: see [README.md](README.md).

feathertune is a Windows-only Tauri 2 app that hosts a music service's official website in a lean WebView2 window. Design, security model, measurements, what is not verified yet and ideas are in [docs/DESIGN.md](docs/DESIGN.md) (pt-BR: [docs/DESIGN.pt-BR.md](docs/DESIGN.pt-BR.md)).

## Layout

- `app/`: the app (Cargo project; run cargo commands from here). `src/main.rs` window, tray, navigation; `src/titlebar.rs`, `src/miniplayer.rs`, `src/gfx.rs` native UI (Direct2D); `src/plugins.rs` the plugin list; `src/settings_window.rs` + `settings/` the settings window; `src/player.rs` player state (the API integrations consume); `src/share_lyrics.rs` lyrics picture cards; `src/smtc.rs` media flyout; `src/injections.rs` + `injections/<id>/` page scripts; `src/privacy.rs` + `filters/` tracker filter; `capabilities/` what the remote page may call; `tauri.conf.json` + `installer-hooks.nsh` the NSIS installer.
- `spike/`: frozen measurement prototype. Do not build it unless a task is about it.
- `measure.ps1`: process-tree meter (Windows, needs a signed-in profile; run by the maintainer). `median.ps1`: median of several of its runs.
- `docs/`: `DESIGN.md`, in English and pt-BR.

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
- Releases: bump `version` in `app/Cargo.toml` and `app/tauri.conf.json` together, then push a `vX.Y.Z` tag; the CI `dist` job publishes the installer. Only when the maintainer asks.
- Anything that needs the running app, a signed-in account or `measure.ps1` is left for the maintainer: say so in the summary instead of claiming it works.

## Code rules

- All code in English: identifiers, comments, file names, `config.toml` keys, logs, commit messages.
- Docs exist in English and pt-BR. When you change one, change its pair (`README.md`/`README.pt-BR.md`, `docs/DESIGN.md`/`docs/DESIGN.pt-BR.md`).
- Match the surrounding style (`rustfmt.toml`: width 120). Minimal changes, no speculative abstractions.
- Every Chromium feature flag goes in the single `--disable-features` in `BROWSER_ARGS` (Chromium only honors the last one).
- UI work (native or HTML) follows the performance rules: draw only on change; any animation has a plugin switch, runs on one timer only while something is on screen to move (stops when the window is hidden, minimized or in the background) and resumes where it stopped; Windows animation effects and `prefers-reduced-motion` do not turn it off (the switch does; the page gets `--force-prefers-no-reduced-motion`); invalidate only the region that changes; keep GPU resources (brushes, layouts, bitmaps) between frames.
- Every plugin is on by default. Write its measured cost in `plugins.rs` and docs/DESIGN.md (or "not measured yet" until the maintainer measures it).
- The page is untrusted. It may only call `player_state`; its payload is validated and never triggers an action. Do not widen `capabilities/` or `allowed()` without a reason written in docs/DESIGN.md.

## Legal and safety limits (never cross)

- Never block, hide, skip or alter ads, and never touch what the player streams. Network filtering is privacy lists only.
- No private APIs, stream extraction, downloading or DRM handling.
- No user-agent spoofing and no automation flags.
- Never type or store account credentials; sign-in and account checks are done by the maintainer.
- Keep brand and product names out of the repo name, UI text, README and docs. Domains and CSS selectors in code are fine where they are functional.
- No personal data in commits: no local paths, usernames, emails, tokens, API keys or listening history. `measure.ps1` output goes to `results/`, which is gitignored; keep it that way.
