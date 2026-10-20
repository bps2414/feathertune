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
        id: "controls",
        name: "Player controls",
        description: "Page module that presses the player's buttons for the tray, media keys, shortcuts and the \
                      mini player. Without it those only show the track.",
        cost: "No separate cost: on in every measurement.",
        default_on: true,
        live: false,
        legacy: Some(("injections", "controls")),
    },
    Plugin {
        id: "tracker_filter",
        name: "Tracker filter",
        description: "Blocks third-party analytics and session-recording requests (privacy lists only, never ads).",
        cost: "Saves memory: -16 MB working set and lower CPU (measured).",
        default_on: true,
        live: false,
        legacy: Some(("privacy", "tracker_filter")),
    },
    Plugin {
        id: "track_notifications",
        name: "Track notifications",
        description: "A notification when the track changes while the window is hidden.",
        cost: "No measurable cost.",
        default_on: true,
        live: true,
        legacy: Some(("notifications", "track_change")),
    },
    Plugin {
        id: "global_shortcuts",
        name: "Global shortcuts",
        description: "Play/pause, next, previous and the mini player from anywhere (keys below).",
        cost: "No measurable cost.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "discord",
        name: "Discord Rich Presence",
        description: "Shows the track on Discord with its artwork, links and a time bar. Needs your Discord application id below.",
        cost: "Not measured: one idle thread, a local message to Discord per track.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "lastfm",
        name: "Last.fm scrobbling",
        description: "Now playing and scrobbles on Last.fm. Connect your account below.",
        cost: "Not measured: one idle thread, two small HTTPS requests per track.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "mini_player",
        name: "Mini player",
        description: "A small always-on-top window with the artwork and the controls; opening it hides the main \
                      window.",
        cost: "Nothing while closed. Open: about the same as the main window in the tray.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "animated_title_bar",
        name: "Animated title bar",
        description: "The colors of the title bar drift slowly. Pauses while the window is hidden, minimized or \
                      in the background, and when Windows animation effects are off.",
        cost: "Not measured yet: the 32 px strip repainted 30 times a second while the window is in front; \
               nothing otherwise.",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "album_color",
        name: "Album color",
        description: "Colors the title bar, the mini player and the site's player with the colors of the current \
                      cover. Taken once per track; text stays readable.",
        cost: "No measurable cost: +1 MB private and +0.1 points of CPU against off, inside the noise.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "synced_lyrics",
        name: "Synced lyrics",
        description: "Shows time-synced lyrics from LRCLIB in the site's Lyrics tab, with the current line \
                      (or word, when the entry has word times) highlighted. One request per track, cached on disk; when LRCLIB has none, the site's own \
                      lyrics stay.",
        cost: "About +5 MB private and WS and +0.2 points of CPU against off (one run), inside the measurement spread.",
        default_on: true,
        live: false,
        legacy: None,
    },
    Plugin {
        id: "ambient_mode",
        name: "Ambient mode",
        description: "A blurred, glowing copy of the cover behind the site. It drifts slowly; paused while the \
                      window is hidden or minimized and with reduced motion.",
        cost: "About +25 MB private and +15 MB WS against off (one run); the CPU difference is inside the noise of one run.",
        default_on: false,
        live: false,
        legacy: None,
    },
];

pub fn get(id: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|p| p.id == id)
}
