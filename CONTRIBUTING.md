# Contributing to feathertune

*Português: [CONTRIBUTING.pt-BR.md](CONTRIBUTING.pt-BR.md)*

Thanks for wanting to help. This guide explains how to send a pull request (PR) that I can review and accept quickly. It is long on purpose: most PRs that come back for changes come back for something written here.

If you use a coding agent (Claude Code, Copilot, Codex...), its instructions are in [AGENTS.md](AGENTS.md). The rules are the same as in this guide.

## Before you start

- **This is a personal project with no formal support.** I read everything, but there is no response time. A PR can wait days for a review.
- **A small, obvious change** (a typo, a clear bug, an interface text): open the PR directly.
- **A big change or one that changes behavior** (a new plugin, a new screen, a new dependency, anything touching audio, lyrics, privacy or what the page may call): open an **issue first** and wait for my answer, so you do not spend a week on something I will not accept.
- **Search first.** Look at the open issues and PRs and at the "Not verified yet" and "Ideas" sections of [docs/DESIGN.md](docs/DESIGN.md). A decision may already be made, or something ruled out with its reason.
- **A bug?** Attach the **Export diagnostics** file (Settings > Advanced). It names the tracks played since the app started; read it before sending and delete what you do not want to publish.

## What I accept and what I do not

These limits come from [AGENTS.md](AGENTS.md) and do not change, not even for a good reason. A PR that crosses any of them is closed.

- **Ads:** never block, hide, skip, shorten, mute, fade or cover them in the repo's code. Every built-in audio effect is bypassed while an ad plays. Network filtering is privacy lists only (third-party analytics). Extensions people import are their own code; the repo never ships, links to or recommends one that touches ads.
- **Audio:** only processed and measured locally, on the page's own sound, for the listener. Never record, store, upload or re-encode audio, and never fetch a stream. Only numbers derived from it (tempo, key, a loudness envelope) may be kept in the profile.
- **No private APIs, stream extraction, downloading or DRM handling.**
- **No user-agent spoofing and no automation flags.**
- **Credentials:** the app never types or stores account credentials. Sign-in and account checks are done by me.
- **The page is untrusted.** It may only call `player_state`, `audio_facts`, `mix_open` and `ext_call`, with validated payloads that never trigger an action on the player. Do not widen `capabilities/` or `allowed()` without writing the reason in `docs/DESIGN.md`.
- **Brand and product names** stay out of the repo name, the UI text, the README and the docs. Domains and CSS selectors in code are fine where they are functional.
- **Personal data** never goes in a commit: no local paths, usernames, emails, tokens, API keys or listening history.

Other things I usually turn down: a new dependency without a clear need (every crate adds build time), a big refactor mixed with a fix, and a feature that costs CPU or memory without a way to turn it off.

## Setting up

The app runs on **Windows 10/11** only (it uses WebView2 and the `windows` crate). It does not build or run on Linux or macOS.

1. Install the WebView2 runtime (already in Windows 11) and Rust **stable** with the MSVC target (Visual Studio Build Tools with "Desktop development with C++").
2. **Fork** the repository on GitHub and clone your fork.
3. Create a branch from `main`, named after the subject: `fix/lyrics-missing-after-ad`, `feat/...`, `docs/...`.
4. Cargo commands run inside `app/`:

```powershell
cd app
cargo check
```

`.cargo/config.toml` already links with `rust-lld`; do not change it. To run the app: `cargo build --release`, then `.\target\release\feathertune.exe`.

The app reads `%LOCALAPPDATA%\feathertune\config.toml`. A local build does not have the Last.fm and Discord keys of the releases; if you need them, put your own in `config.toml` (see the README). You do not need them for most work.

## Build discipline (important)

Builds are slow and I want them not to repeat for nothing. Timings after a one-file change: `cargo check` ~1 s, `cargo build --release` ~9 s, `cargo clippy` ~4 s. A cold build is 1–2 minutes per profile, and `--profile dist` takes several minutes.

- While editing, use **only `cargo check`**.
- Before opening the PR, run **once**: `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked`. This is what CI runs; if it passes here, it passes there.
- Build (`cargo build --release`) only when you are about to **run** the exe, once per batch of changes.
- Use the `release` profile for manual tests. `--profile dist` is only for the installer and release measurements.
- **Never** `cargo clean`, delete `target/`, disable caches or incremental builds, or set `CARGO_INCREMENTAL=0` locally.
- Do not run two builds at once: they block on the same `target/` lock.

## Code rules

- **All code in English:** identifiers, comments, file names, `config.toml` keys, logs and commit messages.
- **Style:** follow the surrounding code. `rustfmt` with width 120 (`app/rustfmt.toml`); `cargo fmt` handles it.
- **Minimal changes.** No speculative abstractions, no file nobody asked for, no "while I am here" edits outside the PR's subject.
- **Dependencies:** do not add a crate or a Cargo feature casually; prefer what is already in `Cargo.toml`. If one is really needed, say in the PR what it gives and what it costs in build time.
- **Chromium feature flags:** every one goes in the single `--disable-features` of `BROWSER_ARGS` (Chromium only honors the last one).
- **Interface texts:** write them in English inside `t("...")` (or `tf` for `{}`) and add the translation to `app/locales/pt-BR.json`. Without an entry the app shows the English, so nothing breaks, but I will ask for the translation. The settings window is translated from the same file.
- **UI and performance** (native or HTML):
  - draw only on change and invalidate only the region that changes;
  - keep GPU resources (brushes, layouts, bitmaps) between frames;
  - any animation has a plugin switch, runs on **one** timer only while something is on screen to move, stops when the window is hidden, minimized or in the background, and resumes where it stopped;
  - Windows animation effects and `prefers-reduced-motion` do not turn it off; the plugin switch does.
- **New plugins:** on by default, except those in the Experimental tab (off until reliable). Write the cost in `plugins.rs` and `docs/DESIGN.md`, or "not measured yet" if you did not measure it. The real measurement (`measure.ps1`) is run by me.
- **Tests:** tests live inside the modules (`#[cfg(test)]`). If you fix a bug in pure logic (lyrics, mixes, privacy, settings), a test that reproduces it is very welcome.
- **Docs in pairs:** docs exist in English and pt-BR. If you change one, change its pair (`README.md` and `README.pt-BR.md`, `docs/DESIGN.md` and `docs/DESIGN.pt-BR.md`, `CONTRIBUTING.md` and `CONTRIBUTING.pt-BR.md`). If you do not write one of the languages, say so in the PR and I will write the other version.
- **Version and CHANGELOG are not yours:** do not change `version` in `Cargo.toml` and `tauri.conf.json`, nor `CHANGELOG.md` and `CHANGELOG.pt-BR.md`, and do not create tags. I make releases. Describe what changed for the user in the PR and I will write the entry.

## What to test before opening

CI only checks formatting, clippy and unit tests, on a Windows runner. It does **not** open the app, play music or have an account. So:

- Run the app (`cargo build --release`) and really use what you changed, on the main path and on an edge case (window hidden, an ad playing, a track change, a first run without `config.toml`).
- In the PR, say **what you tested and how** and **what you did not test**. If something needs a signed-in account, a measurement (`measure.ps1`) or a situation you cannot reproduce, write that instead of saying it works. I check that part.
- Put nothing personal in the PR (text, screenshot, gif or log): usernames, local paths, tokens, emails, account names or what you listen to.
- If you touch the interface, attach a screenshot or a short gif (with no personal data).

## Opening the PR

1. Update your branch with `main` (`git fetch` and rebase) before opening, to avoid conflicts.
2. **One subject per PR.** Two subjects are two PRs; a small PR is reviewed faster.
3. **Commits and title in English**, imperative, starting with the area, as the history already does: `Lyrics: keep the line lit until its end`, `Docs: fix the equalizer table`. Commits that only fix the previous one can be squashed before opening.
4. Open the PR against `main` of the original repository. The form comes with the template (`.github/PULL_REQUEST_TEMPLATE.md`); fill it in. You may write in English or Portuguese.
5. If it is not ready, open it as a **draft**. Take it out of draft only when CI is green and the template is filled.
6. Link the related issue (`Closes #123`) when there is one.

If this is your first PR here, GitHub may ask me to approve the CI run before it starts.

## What happens next

- **CI:** if it goes red, read the log (`fmt`, `clippy` or `test`) and fix it on the same branch with a new push. There is no need to close and open another PR.
- **Review:** I comment on the code. There may be a change request, a question or a refusal with its reason. Answer in the same place. Do not take it personally: refusing is part of keeping the app small and light.
- **Conflicts:** if `main` moved, rebase your branch and push.
- **Merge:** I do the merge, usually as a **squash**, with a message of mine in English. Your name stays as the author of the contribution.
- **A stalled PR:** if I asked for changes and there was no answer for about 30 days, I may close it with a note. You can reopen it whenever you want to continue.
- **Release:** your change reaches users in the next release, with a CHANGELOG entry written by me.

## Using coding agents

You may. The PR is still yours: read the whole diff, run and test what the agent did, and say in the PR that you used an agent. The agent must follow [AGENTS.md](AGENTS.md) (the same build and code rules and the limits above). A PR generated and sent without human review is closed.

## Security

A security problem does not go in a public issue. See [SECURITY.md](SECURITY.md).

## License of contributions

The project uses the [MIT](LICENSE) license. By sending a PR you state that you wrote the code (or have the right to send it) and agree that it is distributed under the MIT license. Do not copy code from a project with an incompatible license or from an unknown source.

## Conduct

Be respectful and direct, with the reviewer and with whoever asks. Criticism is about the code, not the person. Offensive comments, harassment and spam are deleted and the account may be blocked.

## Quick checklist

- [ ] I opened an issue first, if the change is big
- [ ] I do not cross any limit in "What I accept and what I do not"
- [ ] `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked` pass in `app/`
- [ ] I ran the app and tested what I changed; I wrote in the PR what I could not test
- [ ] New interface text is in `t("...")` and in `locales/pt-BR.json`
- [ ] Docs updated in both languages, when I touched one
- [ ] I did not change the version, CHANGELOG or tags
- [ ] Nothing personal in the code, commits, screenshots or logs
- [ ] One subject, commits in English, branch up to date with `main`
