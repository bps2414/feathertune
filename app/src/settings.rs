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
//!
//! [browser]                        # extra Chromium features to turn off, added to the built-in list
//! disable_features = []            # (see main.rs), e.g. ["Prerender2"]
//!
//! [privacy]
//! tracker_filter = true            # privacy.rs; extra rules go in privacy-list.txt in the profile
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
    pub browser: Browser,
    pub privacy: Privacy,
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

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Browser {
    pub disable_features: Vec<String>,
}

impl Browser {
    /// The names that are safe to put in the command line: Chromium feature names are plain
    /// identifiers, and anything else (a space, a comma, a quote) could smuggle in another flag.
    pub fn disabled_features(&self) -> impl Iterator<Item = &str> {
        self.disable_features.iter().map(|f| f.trim()).filter(|f| {
            let ok = !f.is_empty() && f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            if !ok {
                eprintln!("config.toml: ignoring invalid feature name {f:?}");
            }
            ok
        })
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Privacy {
    pub tracker_filter: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Privacy { tracker_filter: true }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_names_cannot_carry_flags() {
        let browser =
            Browser { disable_features: ["Good_1", "a b", "x,y", " ok ", "--evil", ""].map(String::from).to_vec() };
        assert_eq!(browser.disabled_features().collect::<Vec<_>>(), ["Good_1", "ok"]);
    }
}
