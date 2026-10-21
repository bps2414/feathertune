//! Settings, read from the profile's `config.toml` (`%LOCALAPPDATA%\feathertune`, outside the
//! repository, which is where the Last.fm keys and the Discord application id belong: never in code
//! or in Git). The settings window edits the same file through `write`, which keeps the user's
//! comments and layout.
//!
//! ```toml
//! [plugins]                        # one switch per plugin (plugins.rs), all optional
//! tracker_filter = true            # extra rules go in privacy-list.txt in the profile
//!
//! [shortcuts]                      # "" turns one off; syntax: Ctrl+Alt+Space, Ctrl+Shift+Left, ...
//! play_pause = "Ctrl+Alt+Space"
//! next = "Ctrl+Alt+Right"
//! previous = "Ctrl+Alt+Left"
//! mini_player = ""
//!
//! [discord]
//! client_id = ""                   # Rich Presence is off while empty
//!
//! [lastfm]
//! api_key = ""                     # both set: used instead of the keys built into a CI release
//! api_secret = ""
//!
//! [lyrics]
//! providers = ["betterlyrics", "unison", "lrcmux", "lrclib"]  # word-timed ones win, then this order
//! estimate_words = true            # word times for lines timed only as a whole
//! offset_ms = 0                    # moves the lyrics in time; positive: later
//!
//! [browser]                        # extra Chromium features to turn off, added to the built-in list
//! disable_features = []            # (see main.rs), e.g. ["Prerender2"]
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::Deserialize;

use crate::plugins::{self, PLUGINS};

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub shortcuts: Shortcuts,
    pub discord: Discord,
    pub lastfm: LastFm,
    pub browser: Browser,
    pub lyrics: LyricsSettings,
    /// The whole file, for the plugin switches (`plugin`).
    #[serde(skip)]
    raw: toml::Table,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub play_pause: String,
    pub next: String,
    pub previous: String,
    pub mini_player: String,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts {
            play_pause: "Ctrl+Alt+Space".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
            mini_player: String::new(),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Discord {
    pub client_id: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct LastFm {
    pub api_key: String,
    pub api_secret: String,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct LyricsSettings {
    /// Source ids (`lyrics::PROVIDERS`); unknown ones are skipped.
    pub providers: Vec<String>,
    pub estimate_words: bool,
    pub offset_ms: i32,
}

impl Default for LyricsSettings {
    fn default() -> Self {
        LyricsSettings {
            providers: crate::lyrics::PROVIDERS.iter().map(|(id, _)| id.to_string()).collect(),
            estimate_words: true,
            offset_ms: 0,
        }
    }
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
            let ok = valid_feature(f);
            if !ok {
                eprintln!("config.toml: ignoring invalid feature name {f:?}");
            }
            ok
        })
    }
}

pub fn valid_feature(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Settings {
    /// Missing file: writes one with every plugin at its default and uses the defaults. A file that
    /// does not parse is reported, left untouched, and the defaults are used.
    pub fn load(profile: &Path) -> Settings {
        let path = profile.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => Settings::parse(&text).unwrap_or_else(|e| {
                eprintln!("config.toml is invalid, using defaults: {e}");
                Settings::default()
            }),
            Err(_) => {
                let _ = std::fs::create_dir_all(profile);
                let _ = std::fs::write(&path, default_file());
                Settings::default()
            }
        }
    }

    pub fn parse(text: &str) -> Result<Settings, toml::de::Error> {
        let raw: toml::Table = toml::from_str(text)?;
        let mut settings: Settings = toml::from_str(text)?;
        settings.raw = raw;
        Ok(settings)
    }

    /// The plugin's switch: `[plugins]`, then its old place, then its default. Unknown ids are off.
    pub fn plugin(&self, id: &str) -> bool {
        let Some(plugin) = plugins::get(id) else { return false };
        let switch = |table: &str, key: &str| self.raw.get(table).and_then(|t| t.get(key)).and_then(|v| v.as_bool());
        switch("plugins", id)
            .or_else(|| plugin.legacy.and_then(|(table, key)| switch(table, key)))
            .unwrap_or(plugin.default_on)
    }
}

/// The first `config.toml`: every plugin with its description, at its default.
fn default_file() -> String {
    let mut text = String::from(
        "# feathertune settings. The settings window (title bar menu > Settings) edits this file and keeps\n\
         # your comments. Every key is optional; see the README for the full list.\n\n[plugins]\n",
    );
    for p in PLUGINS {
        text.push_str(&format!("# {}: {}\n{} = {}\n", p.name, p.description, p.id, p.default_on));
    }
    text
}

/// The settings shared by the running app and the settings window.
pub struct Live {
    pub profile: PathBuf,
    /// The WebView2 command line. Every webview of the app must use the same one (and the same
    /// profile) to share one browser process.
    pub browser_args: String,
    current: Mutex<Settings>,
    /// A change was saved that only applies after a restart.
    restart: AtomicBool,
}

impl Live {
    pub fn new(profile: PathBuf, browser_args: String, settings: Settings) -> Live {
        Live { profile, browser_args, current: Mutex::new(settings), restart: AtomicBool::new(false) }
    }

    pub fn get(&self) -> MutexGuard<'_, Settings> {
        self.current.lock().unwrap()
    }

    pub fn plugin(&self, id: &str) -> bool {
        self.get().plugin(id)
    }

    pub fn restart_needed(&self) -> bool {
        self.restart.load(Ordering::Relaxed)
    }

    /// Sets `[table] key = value` in `config.toml`, keeping everything else (comments included),
    /// removes `remove` (an old place of the same switch) and reloads the settings. A file that does
    /// not parse is not written.
    pub fn write(
        &self,
        table: &str,
        key: &str,
        value: toml_edit::Item,
        remove: Option<(&str, &str)>,
        restart: bool,
    ) -> Result<(), String> {
        let path = self.profile.join("config.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("could not read config.toml: {e}")),
        };
        let text = edit(&text, table, key, value, remove)?;
        let settings = Settings::parse(&text).map_err(|e| format!("the new config.toml would not parse: {e}"))?;
        std::fs::write(&path, &text).map_err(|e| format!("could not write config.toml: {e}"))?;
        *self.get() = settings;
        if restart {
            self.restart.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

fn edit(
    text: &str,
    table: &str,
    key: &str,
    value: toml_edit::Item,
    remove: Option<(&str, &str)>,
) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|e| format!("config.toml has an error; fix it by hand first: {e}"))?;
    if let Some((t, k)) = remove
        && let Some(old) = doc.get_mut(t).and_then(|i| i.as_table_like_mut())
    {
        old.remove(k);
    }
    let section = doc.entry(table).or_insert_with(toml_edit::table);
    let Some(section) = section.as_table_like_mut() else {
        return Err(format!("config.toml: `{table}` is not a table"));
    };
    match section.get_mut(key) {
        // Keeps the comment that follows the old value on the same line.
        Some(toml_edit::Item::Value(old)) if value.is_value() => {
            let decor = old.decor().clone();
            *old = value.into_value().expect("checked");
            *old.decor_mut() = decor;
        }
        _ => {
            section.insert(key, value);
        }
    }
    Ok(doc.to_string())
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

    #[test]
    fn plugin_switch_falls_back_to_the_old_key_then_the_default() {
        let s = Settings::parse("[injections]\ncontrols = false\n[privacy]\ntracker_filter = false").unwrap();
        assert!(!s.plugin("controls"));
        assert!(!s.plugin("tracker_filter"));
        assert!(s.plugin("track_notifications"));
        assert!(!s.plugin("nonexistent"));
        let s = Settings::parse("[plugins]\ncontrols = true\n[injections]\ncontrols = false").unwrap();
        assert!(s.plugin("controls"));
    }

    #[test]
    fn edit_keeps_comments_and_moves_old_keys() {
        let text = "# mine\n[privacy]\ntracker_filter = true # keep?\n\n[shortcuts]\nnext = \"Ctrl+N\" # my key\n";
        let out = edit(text, "plugins", "tracker_filter", toml_edit::value(false), Some(("privacy", "tracker_filter")))
            .unwrap();
        let out = edit(&out, "shortcuts", "next", toml_edit::value("Ctrl+M"), None).unwrap();
        assert!(out.starts_with("# mine\n"));
        assert!(out.contains("next = \"Ctrl+M\" # my key"));
        assert!(!out.contains("[privacy]\ntracker_filter"));
        let s = Settings::parse(&out).unwrap();
        assert!(!s.plugin("tracker_filter"));
        assert_eq!(s.shortcuts.next, "Ctrl+M");
    }

    #[test]
    fn default_file_lists_every_plugin() {
        let s = Settings::parse(&default_file()).unwrap();
        for p in PLUGINS {
            assert_eq!(s.plugin(p.id), p.default_on, "{}", p.id);
        }
    }
}
