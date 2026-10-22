<!-- Read CONTRIBUTING.md first (pt-BR: CONTRIBUTING.pt-BR.md). You may write in English or Portuguese. -->

## What and why

<!-- What changes for the user, and why. Link the issue: Closes #123 -->

## How I tested

<!-- What you ran in the real app and how (main path and an edge case: window hidden, an ad playing, a track change, a first run). -->

## What I did not test

<!-- Anything that needs a signed-in account, measure.ps1 or a case you could not reproduce. Say so instead of claiming it works. -->

## Screenshots or gif

<!-- UI changes only. No personal data (usernames, paths, account names, what you listen to). -->

## Checklist

- [ ] One subject; commits and title in English (`Area: what changed`)
- [ ] `cargo fmt`, `cargo clippy --all-targets --locked -- -D warnings` and `cargo test --locked` pass in `app/`
- [ ] No limit in AGENTS.md is crossed (ads, audio, private APIs, credentials, what the page may call, brand names)
- [ ] New interface text is in `t("...")` and in `app/locales/pt-BR.json`
- [ ] New plugin: on by default (or Experimental), cost written in `plugins.rs` and `docs/DESIGN.md` ("not measured yet" is fine)
- [ ] Docs changed in both languages (EN and pt-BR)
- [ ] I did not change the version, the CHANGELOG or tags
- [ ] Nothing personal in the code, commits, screenshots or logs
- [ ] If an AI agent wrote part of this, I read the whole diff and say so here
