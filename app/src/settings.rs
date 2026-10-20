//! Settings of the Phase 3 integrations, read from the profile's `config.toml` (the same file as the
//! `[injections]` switches). It lives in `%LOCALAPPDATA%\feathertune`, outside the repository, which is
//! where the Last.fm keys and the Discord application id belong: never in code or in Git.
//!
//! ```toml
//! [shortcuts]                      # "" turns one off; syntax: Ctrl+Alt+Space, Ctrl+Shift+Left, ...
//! play_pause = "Ctrl+Alt+Space"
//! next = "Ctrl+Alt+Right"
//! previous = "Ctrl+Alt+Left"
//!
//! [notifications]
//! track_change = true              # toast on track change, only while the window is hidden
//!
//! [discord]
//! client_id = ""                   # Rich Presence is off while empty
//!
//! [lastfm]
//! api_key = ""                     # off while either is empty
//! api_secret = ""
//! ```

use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub shortcuts: Shortcuts,
    pub notifications: Notifications,
    pub discord: Discord,
    pub lastfm: LastFm,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub play_pause: String,
    pub next: String,
    pub previous: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts {
            play_pause: "Ctrl+Alt+Space".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub track_change: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications { track_change: true }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Discord {
    pub client_id: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LastFm {
    pub api_key: String,
    pub api_secret: String,
}

impl Settings {
    /// Missing file or sections give the defaults. A file that does not parse is reported by
    /// `injections::Config::load` and is not touched here either.
    pub fn load(profile: &Path) -> Settings {
        std::fs::read_to_string(profile.join("config.toml"))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }
}
