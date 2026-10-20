# Changelog

What changed in each release of feathertune. Installers are on the [releases page](https://github.com/bps2414/feathertune/releases). (pt-BR: [CHANGELOG.pt-BR.md](CHANGELOG.pt-BR.md))

## 0.1.7 — 2026-10-20

- **Long playlists no longer fade out.** With ambient mode on, the track list of a long playlist or album faded to nothing a few rows down the page. It scrolls normally again.

## 0.1.6 — 2026-10-20

### Mini player
- **Lyrics roll smoothly.** On a line change the next line rises into place and grows, the old one fades out upward and the new next line comes in from below. Lines no longer sit on top of each other.
- **Bigger, clearer buttons.** Play/pause is a large disc, previous and next have larger targets, and the top buttons (lyrics, full window, close) sit on soft pills and say what they do when you hover them.

### Share lyrics
- **A prettier card.** The album's main color tints the quote mark, a bar beside the lines and the skip dots. The lines get a soft shadow, the cover a light rim, Ink and Dark a vignette, and every look a fine film grain.
- **A real music note** in the footer, drawn on a tinted disc, instead of the old font glyph.

## 0.1.5 — 2026-10-20

- **Share button in the title bar**, shown only while the track has synced lyrics.
- **Nicer share card:** large quote mark, soft sheen, glass edge, bigger cover, and a footer with the album. No app name on the card.
- **Lyrics in the mini player:** a new button adds a band with the current line and the next one.
- **Tray menu fix:** it no longer stays open after you click somewhere else.

## 0.1.4 — 2026-10-20

- **Share lyrics:** pick any lines (up to 8, Shift-click for a range) and get a picture card in four looks: Cover, Ink, Dark and Paper. Copy it to the clipboard or save it as a 1080 px PNG.
- **Floating lyrics:** the current and next line float over the desktop, click-through when locked.
- **Taskbar controls:** previous, play/pause and next on the taskbar thumbnail, plus track progress on the button.
- **Themed tray menu** in the album's colors.
- **Listening stats:** local history with a Stats tab in the settings and CSV export.
- **Lyrics scroll** parks on the current line and moves on around the switch, so a line that is ending is not already high up.

## 0.1.3 — 2026-10-20

- **Lyrics:** lines cross-fade, and while a song plays the list scrolls in time with it, easing after seeks, clicks and manual scrolls.
- **Animations** follow their own switches in the settings, not Windows' animation effects.
- **Ambient mode** is on by default, like every plugin.
- **Fix:** the site's backdrop no longer repeats as a line every half screen in ambient mode.

## 0.1.2 — 2026-10-20

- **Start screen** that shows what the app is doing while it checks for updates and preloads the site.
- **Self-update** from the latest release: downloaded, checked against its SHA-256 and installed silently, then the app restarts. Only installed copies update themselves.
- **Lyrics:** larger text, lines fade by distance from the current one, click a line to seek.
- **Fix:** the settings' "Restart now" bar was always visible.

## 0.1.1 — 2026-10-20

- **Fix: Last.fm scrobbling.** The scrobbler stopped at its first deadline, so nothing was scrobbled.
- **Lyrics:** larger text, and entries with word timing light up word by word.
- **Discord:** richer presence with the track's artwork, links and a button.
- **Ambient mode:** the top backdrop fades out cleanly; solid search box.

## 0.1.0 — 2026-10-20

First release.

- The site's official web player in a lean WebView2 window, with media keys and a tray icon.
- **Native title bar** in a liquid-glass look tinted by the album, and a **native mini player**, always on top.
- **Settings window** with a switch for every plugin and shortcut fields.
- **Album colors** on the title bar, mini player and page, plus an optional **ambient mode**.
- **Synced lyrics** from LRCLIB in the site's Lyrics tab.
- **Last.fm** scrobbling (with an offline queue) and **Discord** Rich Presence.
- **Privacy filter** for trackers (privacy lists only; ads and playback are never touched).
- **Per-user installer** that can also delete your data on uninstall if you tick the box.
