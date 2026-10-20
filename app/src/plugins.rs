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
    /// What it costs, from the plan's measurements (docs/PLAN.md).
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
        cost: "No separate cost: on in every measurement since Phase 2.",
        default_on: true,
        live: false,
        legacy: Some(("injections", "controls")),
    },
    Plugin {
        id: "tracker_filter",
        name: "Tracker filter",
        description: "Blocks third-party analytics and session-recording requests (privacy lists only, never ads).",
        cost: "Saves memory: -16 MB working set and lower CPU (Phase 4, E6).",
        default_on: true,
        live: false,
        legacy: Some(("privacy", "tracker_filter")),
    },
    Plugin {
        id: "track_notifications",
        name: "Track notifications",
        description: "A notification when the track changes while the window is hidden.",
        cost: "No measurable cost (Phase 3 rows).",
        default_on: true,
        live: true,
        legacy: Some(("notifications", "track_change")),
    },
    Plugin {
        id: "global_shortcuts",
        name: "Global shortcuts",
        description: "Play/pause, next, previous and the mini player from anywhere (keys below).",
        cost: "No measurable cost (Phase 3 rows).",
        default_on: true,
        live: true,
        legacy: None,
    },
    Plugin {
        id: "discord",
        name: "Discord Rich Presence",
        description: "Shows the track as \"Listening to\" on Discord. Needs your Discord application id below.",
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
        cost: "Nothing while closed. Open: see the mini player row in the plan.",
        default_on: true,
        live: true,
        legacy: None,
    },
];

pub fn get(id: &str) -> Option<&'static Plugin> {
    PLUGINS.iter().find(|p| p.id == id)
}
