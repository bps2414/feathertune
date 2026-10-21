//! Plugins: every optional feature of the app (a page injection, a Rust integration, or both) is
//! declared here with an id, a name, a description, its measured cost and a default. The switch is
//! `[plugins] <id> = true/false` in the profile's `config.toml`; the settings window lists every
//! entry of `PLUGINS` with its switch. Plugins are compiled in: nothing is loaded from a DLL or
//! downloaded.
//!
//! To add one: add an entry here, then check `Settings::plugin("<id>")` where the feature starts
//! (and, for a page module, set `plugin` in `injections::INJECTIONS`).

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// What it costs, from the measurements in docs/DESIGN.md.
    pub cost: &'static str,
    pub default_on: bool,
    /// The switch takes effect at once; otherwise on the next start.
    pub live: bool,
    /// Where the switch lived before the `[plugins]` table (`table`, `key`). Still read when
    /// `[plugins]` does not name the plugin, and removed when the settings window writes it.
    pub legacy: Option<(&'static str, &'static str)>,
}

pub const PLUGINS: &[Plugin] = &[
    Plugin {
        id: "launcher",
        name: "Launcher",
        description: "A start screen that says what the app is doing while it opens. It checks for a newer \
                      release and installs it (the app restarts), then shows the window once the player has \
                      loaded. Without it the window opens at once and nothing is checked.",
        cost: "Not measurable here (off under --measure; its row, +2.7 points of CPU and -5 MB private, is the noise of \
               the method). One small request to GitHub at each start.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "controls",
        name: "Player controls",
        description: "Page module that presses the player's buttons for the tray, media keys, shortcuts and the \
                      mini player. Without it those only show the track.",
        cost: "Inside the noise: +2.0 points of CPU and -3 MB private against off (3 runs).",
        default_on: true,
        live: false,
        legacy: Some(("injections", "controls")),
    },
    Plugin {
        id: "tracker_filter",
        name: "Tracker filter",
        description: "Blocks third-party analytics and session-recording requests (privacy lists only, never ads).",
        cost: "Saves memory: -8 MB private and -8 MB WS against off (3 runs); CPU inside the noise.",
        default_on: true,
        live: false,
        legacy: Some(("privacy", "tracker_filter")),
    },
    Plugin {
        id: "track_notifications",
        name: "Track notifications",
        description: "A notification when the track changes while the window is hidden.",
        cost: "Inside the noise: +0.2 points of CPU and -3 MB private against off (3 runs).",
        default_on: true,
        live: true,
        legacy: Some(("notifications", "track_change")),
    },
    Plugin {
        id: "global_shortcuts",
        name: "Global shortcuts",
        description: "Play/pause, next, previous and the mini player from anywhere (keys below).",
        cost: "Inside the noise: +0.8 points of CPU and -4 MB private against off (3 runs).",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "discord",
        name: "Discord Rich Presence",
        description: "Shows the track on Discord with its artwork, links and a time bar. Needs your Discord application id below.",
        cost: "Inside the noise (3 runs), but the profile had no application id, so nothing ran: one idle thread, a \
               local message to Discord per track (not measured with an id).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "lastfm",
        name: "Last.fm scrobbling",
        description: "Now playing and scrobbles on Last.fm. Connect your account below.",
        cost: "Inside the noise (3 runs), but the profile had no keys or account, so nothing ran: one idle thread, two \
               small HTTPS requests per track (not measured connected).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "mini_player",
        name: "Mini player",
        description: "A small always-on-top window with the artwork and the controls; opening it hides the main \
                      window. Its lyrics button adds the current line of the synced lyrics and the next one (when \
                      Synced, Floating or Share lyrics is on).",
        cost: "Closed: inside the noise (+2.6 points of CPU, -2 MB private against off, 3 runs). Open, main window \
               hidden: 4.5% CPU, 518 MB private, 227 MB WS, the same as the tray.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "themed_tray_menu",
        name: "Themed tray menu",
        description: "The tray icon's right-click menu in the app's look, with the track and its controls on \
                      top. Off: the Windows menu, which screen readers read as a menu.",
        cost: "Inside the noise while closed: -2.2 points of CPU and -4 MB private against off (3 runs); a small \
               window while open.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "taskbar_controls",
        name: "Taskbar controls",
        description: "Previous, play/pause and next on the window's taskbar thumbnail, and the track's progress \
                      on its taskbar button (yellow while paused).",
        cost: "Inside the noise: +1.5 points of CPU and +1 MB private against off (3 runs); a progress update every \
               1 to 5 s while a track plays and the window is shown.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "desktop_lyrics",
        name: "Floating lyrics",
        description: "The current line of the synced lyrics (and the next one) over the desktop, in a window that \
                      clicks pass through. Open it from the tray or below; unlock it in the tray to move it. \
                      Uses the same lyrics sources as Synced lyrics, even with that one off.",
        cost: "Closed: inside the noise (+3.4 points of CPU, +1 MB private against off, 3 runs). Open: 5.6% CPU and \
               519 MB private (+10 MB against base; the CPU is not comparable with base, see docs/DESIGN.md).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "listening_stats",
        name: "Listening stats",
        description: "Keeps a history of what you play (only on this PC, in the profile) and shows it in the \
                      Stats tab: listening time, top artists, tracks and albums, when you listen and streaks.",
        cost: "Inside the noise: 0.0 points of CPU and -2 MB private against off (3 runs); one idle thread and one \
               line appended to a file per track.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "share_lyrics",
        name: "Share lyrics",
        description: "Makes a picture card of the synced lyrics: pick any lines (skipped ones are marked), choose \
                      a look (cover colors, album ink, dark or paper), then copy it or save it as a PNG. Open it \
                      from the title bar, the tray or below. Uses the same lyrics sources as Synced lyrics, even \
                      with that one off.",
        cost: "Closed: inside the noise (-0.2 points of CPU, +1 MB private against off, 3 runs). Open: 5.2% CPU and \
               535 MB private (+26 MB against base; the CPU is not comparable with base, see docs/DESIGN.md).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "animated_title_bar",
        name: "Animated title bar",
        description: "The colors of the title bar drift slowly. Pauses while the window is hidden, minimized or \
                      in the background.",
        cost: "Inside the noise: +3.8 points of CPU and +1 MB private against off (3 runs); the 32 px strip repaints \
               30 times a second while the window is in front.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "album_color",
        name: "Album color",
        description: "Colors the title bar, the mini player and the site's player with the colors of the current \
                      cover. Taken once per track; text stays readable.",
        cost: "Inside the noise: +2.6 points of CPU and +1 MB private against off (3 runs).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "synced_lyrics",
        name: "Synced lyrics",
        description: "Shows time-synced lyrics in the site's Lyrics tab, from four open sources (below), with \
                      the current line and word highlighted; sources timed word by word win, and word times are \
                      estimated when none has them. Lines cross-fade and the list scrolls slowly in time with \
                      the song while it plays and the window is in front. A few requests per new track, at \
                      once, cached on disk; when no source has any, the site's own lyrics stay.",
        cost: "Tab closed: inside the noise (+3.8 points of CPU, -1 MB private against off, 3 runs). Lyrics tab open: \
               the largest cost measured, 55% CPU against 11% with the plugin off (+44 points), +338 MB private and \
               +330 MB WS.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "ambient_mode",
        name: "Ambient mode",
        description: "A blurred, glowing copy of the cover behind the site. It drifts slowly; paused while the \
                      window is hidden or minimized.",
        cost: "+56 MB private and +8 MB WS against off, and +4.3 points of CPU (3 runs).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "navigation",
        name: "Back and forward",
        description: "Back and forward buttons on the title bar, dimmed when there is nowhere to go. They use the \
                      browser's own history (Alt+Left and Alt+Right work too).",
        cost: "Inside the noise: +0.3 points of CPU and -4 MB private against off (3 runs).",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "song_mode",
        name: "Song mode",
        description: "When a track comes in both a song and a video version, picks the song one (the site's own \
                      Song / Video switch), so the video is not decoded. Choosing Video yourself holds until the \
                      next track.",
        cost: "Inside the noise: +1.6 points of CPU and +1 MB private against off (3 runs); the saving while a track \
               with a video plays as the song is not measured.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "precise_volume",
        name: "Precise volume",
        description: "The mouse wheel over the player bar or the expanded player changes the volume by 2% (10% \
                      with Shift), with a small percentage that fades. Optional global keys under Shortcuts.",
        cost: "Inside the noise: -0.4 points of CPU and -2 MB private against off (3 runs).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "skip_disliked",
        name: "Skip disliked tracks",
        description: "Skips a track that starts already marked with the site's dislike. Never during an ad, at \
                      most one skip per track and five in a row.",
        cost: "Inside the noise: +1.1 points of CPU and -4 MB private against off (3 runs).",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "equalizer",
        name: "Equalizer",
        description: "A 10-band equalizer with presets (Audio tab), on the page's own audio and only on this PC. \
                      Flat (the default) leaves it out of the sound path. Every audio effect is bypassed while an \
                      ad plays.",
        cost: "Inside the noise with flat bands: +3.9 points of CPU and -3 MB private against off (3 runs); the \
               Web Audio graph is shared by every audio plugin and bands that are not flat are not measured.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "compressor",
        name: "Compressor",
        description: "Evens out loud and quiet parts (soft, night or strong; Audio tab). \"None\" (the default) \
                      leaves it out of the sound path. Bypassed while an ad plays.",
        cost: "Inside the noise: +1.2 points of CPU and -5 MB private against off (3 runs).",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "crossfade",
        name: "Crossfade",
        description: "Fades each track out into the next (6 s, equal power by default; Audio tab). The site still \
                      picks the next track, so repeat, shuffle and autoplay work as usual. Never over or into an \
                      ad, never on repeat-one.",
        cost: "Inside the noise: -1.3 points of CPU and -1 MB private against off (3 runs); the transition itself is \
               not measured.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "track_analysis",
        name: "Track analysis",
        description: "The first time a track plays from its start, measures its tempo, key (an estimate), where \
                      its sound starts and ends, and a loudness outline, on this PC only. Only those numbers are \
                      kept (in the profile), never audio. The crossfade, skip silence and the Mix tab use them.",
        cost: "Inside the noise: +2.9 points of CPU and -2 MB private against off (3 runs); the first minute of a \
               track not measured yet is not isolated.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "skip_silence",
        name: "Skip silence",
        description: "Skips silence at the start and the end of a track (never in the middle), using the track \
                      analysis when there is one. Never during an ad or after you seek in the track.",
        cost: "Inside the noise: +0.9 points of CPU and -3 MB private against off (3 runs).",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "visualizer",
        name: "Visualizer",
        description: "Bars or a wave of the music under the artwork on the expanded player page, in the album's \
                      colors (Audio tab). Hidden while an ad plays.",
        cost: "Inside the noise with the expanded player closed: -1.1 points of CPU and -6 MB private against off \
               (3 runs); drawing with it open is not measured.",
        default_on: true,
        live: true,
        legacy: None,
    },
];

pub fn get(id: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|p| p.id == id)
}
