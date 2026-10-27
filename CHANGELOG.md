# Changelog

What changed in each release of feathertune. Installers are on the [releases page](https://github.com/bps2414/feathertune/releases). (pt-BR: [CHANGELOG.pt-BR.md](CHANGELOG.pt-BR.md))

## 0.5.1 — 2026-10-27

- **Playing a playlist from the phone no longer gets stuck.** The PC showed a "Leave site?" dialog and waited on it; the playlist now just starts.
- **Tap a track in the phone's queue to play it.** The tap reached the PC but the track did not start; it does now.
- **Play next moves the track.** It used to leave the track where it was and add a copy after the playing one.
- **No more dragging in the phone's queue.** The site ignored the drag; use a track's menu for Play next and Remove. Android APK: `feathertune-remote-0.2.1.apk`, attached to this release; it installs over 0.2.0.

## 0.5.0 — 2026-10-26

- **Phone remote: a new look.** The player sits on a gradient from the album's colors, with an "up next" bar, a Player and a Library tab, and the queue in a sheet. The app now has its own icon (the PC app's feather) and a start screen.
- **Live volume from the phone.** The PC's volume follows the slider while you drag it, not only when you let go.
- **Reorder the queue from the phone.** Drag a track by its handle, or use a track's menu for Play next and Remove.
- **Play a playlist of your library from the phone.** The Library tab lists the playlists of the site's sidebar; tap one to play it on the PC.
- The remote protocol is now version 2. The 0.1.0 phone app still works with this PC; the new phone app hides queue editing and the library on an older PC and asks to update it. Android APK: `feathertune-remote-0.2.0.apk`, attached to this release; it installs over 0.1.0.

## 0.4.1 — 2026-10-24

- **Enable Lyrics tab when external lyrics are found.** When YouTube Music says a track has no lyrics (the Lyrics tab button is grayed out/disabled), but feathertune finds synchronized lyrics from external sources, the tab button is now automatically enabled and clickable so you can open and view the lyrics.

## 0.4.0 — 2026-10-23

- **Phone remote control (highly experimental preview).** Control feathertune playback on a paired PC over the local network (Now Playing, Queue with tap to play, seek, volume, play/pause, next/previous). End-to-end encrypted with the Noise Protocol (`Noise_IKpsk2` pairing via one-time QR code confirmed on the PC, `Noise_KK` for later sessions). Off by default in Settings > Experimental.
- **Android remote APK attached.** An early, highly experimental standalone Android build (`feathertune-remote-0.1.0.apk`) is attached to this release for testing and feedback.

## 0.3.1 — 2026-10-23

- **Synced lyrics on every track of a queue.** When tracks played back to back, the song length and time kept counting from the first track, so lyrics were looked up with the wrong length and often not found, and crossfade, skip silence and track analysis worked only on the first track. Every feature now uses the track's own time.
- **Lyrics tab before the first line.** The first lines start at the top of the tab instead of sitting under an empty half, and the lines to come are lit as the next ones.
- **Game mode button.** While on, it is green with a dot under it, also with the pointer over it. The floating lyrics no longer reopen when game mode is turned off and on again quickly.

## 0.3.0 — 2026-10-22

- **Lyrics tab without lag.** The lyrics now glide on the graphics side, on their own clock in time with the song, so a busy page no longer makes them stutter or drop frames. A line that ends no longer repaints itself, the next lines are prepared before they come on screen, and ambient mode's background holds still while the lyrics show. Same look, same speed.
- **Continuous scroll on fast songs.** In fast parts the Lyrics tab flows at the part's pace instead of speeding up and slowing down with each line.
- **New icon.** A feather.

## 0.2.7 — 2026-10-22

- **Overlapping lyrics on fast songs.** When a second voice or an echo starts before the line before it ends, both lines now stay lit and light their own words until each one is done, instead of the first being cut off at the start of the next. Each line keeps its end time from the source.
- **Background vocals.** Lyrics with background vocals show them as a smaller line under the main one, lit word by word, instead of leaving them out.
- **Smoother Lyrics tab.** Lines now fade and grow without being painted again, and the glow (same look, same album color) is a layer of its own, so a line change or a lighting word no longer repaints a blur. Lyrics cached before this release are fetched again once to get the line ends.

## 0.2.6 — 2026-10-21

- **Synced lyrics no longer go missing after a track change.** At an automatic track change or after an ad the lyrics were looked up with the wrong song length, so no source matched and the track stayed without synced lyrics until it was played again. The lookup now waits until the track's details settle, asks again if the length changes, and a slow lookup no longer holds up the next track.
- **Translations come sooner.** They no longer wait for the slowest lyrics source.
- **Smoother Lyrics tab.** The stutter when the next line lights up and the scroll comes to rest is gone, and the scroll glides without 1-pixel steps.
- **Game mode.** A controller button on the title bar turns off at once the features measured as the costliest (synced lyrics in the page, ambient mode, the floating and share lyrics windows); a second click brings them back.
- **Export diagnostics** (Settings > Advanced): a text file with what the app did, to send when something goes wrong.

## 0.2.5 — 2026-10-21

- **Word-by-word lyrics on fast songs.** Words light right when they are sung, also with the window not in front, and fast words light at once instead of fading in behind the voice. The Lyrics tab scrolls at a steady pace through fast lines instead of stopping at each one, and no longer trails the current line.

## 0.2.4 — 2026-10-21

- **The app in Portuguese (Brazil).** Menus, mini player, floating and shared lyrics, the start screen, the settings and What's new follow Windows' language; pick English or Português in the General tab of the settings (applies after a restart).

## 0.2.3 — 2026-10-21

- **Lyrics translation.** A translation under each synced line, in Windows' language by default (or one picked in the Lyrics tab of the settings): the source's own translation when it has one, else a machine translation made for lyrics. The original still lights word by word; the translation lights with its line. The floating lyrics and the mini player show it in place of the next line. Lines already in that language get none, and it can be turned off.

## 0.2.2 — 2026-10-21

- **Discord Rich Presence works out of the box.** The release carries its own Discord application id, so there is nothing to set up; an id typed in the settings still takes its place.

## 0.2.1 — 2026-10-21

- **The window opens after the start screen, maximized.** It could stay in the tray, or come up at its normal size when it had been maximized.

## 0.2.0 — 2026-10-21

**Experimental release.** The new audio features (crossfade, mixes, skip silence, track analysis) are still rough and may misbehave, so they start off, in the new Experimental tab of the settings. Everything else is on as usual.

### Lyrics
- **Word by word more often.** Two more open sources timed word by word, all sources asked at once: the first answer shows right away and a word-timed one replaces it when it comes.
- **Estimated word timing** for lyrics timed only by line, from syllables and the song's own pace (it can be turned off), plus a timing offset.
- **A Lyrics tab** in the settings to pick and order the sources.

### Audio
- **Equalizer** (10 bands, presets) and **compressor** (soft, night, strong), on this PC only. Every effect is bypassed while an ad plays.
- **Visualizer:** bars or a wave under the artwork on the expanded player.
- **Experimental:** crossfade with curves, fade-in and echo out; skip silence at the start and end of a track; local track analysis (tempo, key, loudness); mixes per playlist and per pair of tracks, with overlapping transitions when the account allows it.

### Player
- **Back and forward** buttons on the title bar.
- **Song mode:** picks the song version of a track over the video, so no video is decoded.
- **Precise volume:** the mouse wheel over the player changes the volume by 2% (10% with Shift), with optional global keys.
- **Skip disliked tracks** that start already marked with a dislike.

### Window and settings
- **Fullscreen** from the player fills the window and leaves cleanly; the page lays out again after a display or DPI change. "Reload page" in the tray.
- **The settings tabs are a sidebar**, so every tab is in sight, plus a search over all of them.
- **What's new:** after an update the settings open on this list.

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
