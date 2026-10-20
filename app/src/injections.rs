//! Page injections: every piece of JS and CSS that enters the page lives in `injections/<id>/`
//! (`page.js` and/or `style.css`) and is compiled into the binary.
//!
//! Each module has an on/off switch under `[injections]` in the profile's `config.toml`, except the
//! `core` module (the player state bridge), which is always on. A module that is an unmeasured
//! experiment gets `default_on: false` until a measurement shows a gain.
//! On startup the enabled modules are assembled into a single initialization script that only
//! runs on the main frame of https://music.youtube.com. CSS enters as a `<style>` element and each
//! module runs inside its own `try/catch`, so a module that throws does not take the others down.
//! Modules share the `feathertune` namespace object (`window.__feathertune`), which is how Rust reaches them.
//!
//! To add a module: create its folder and add one entry to `INJECTIONS`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

struct Injection {
    id: &'static str,
    js: Option<&'static str>,
    css: Option<&'static str>,
    /// false: always on, with no switch in `config.toml`.
    toggle: bool,
    /// State when `config.toml` does not mention the module.
    default_on: bool,
}

const INJECTIONS: &[Injection] = &[
    Injection {
        id: "core",
        js: Some(include_str!("../injections/core/page.js")),
        css: None,
        toggle: false,
        default_on: true,
    },
    Injection {
        id: "controls",
        js: Some(include_str!("../injections/controls/page.js")),
        css: None,
        toggle: true,
        default_on: true,
    },
];

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    injections: BTreeMap<String, bool>,
}

impl Config {
    /// Reads `config.toml` from the profile. Writes a default one (every module at its default) if it is missing.
    /// A file that does not parse is left untouched and the defaults are used.
    pub fn load(profile: &Path) -> Config {
        let path = profile.join("config.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                eprintln!("config.toml is invalid, using defaults: {e}");
                Config::default()
            }),
            Err(_) => {
                let mut text = String::from("[injections]\n");
                for inj in INJECTIONS.iter().filter(|i| i.toggle) {
                    text.push_str(&format!("{} = {}\n", inj.id, inj.default_on));
                }
                let _ = std::fs::create_dir_all(profile);
                let _ = std::fs::write(&path, text);
                Config::default()
            }
        }
    }

    fn enabled(&self, inj: &Injection) -> bool {
        !inj.toggle || self.injections.get(inj.id).copied().unwrap_or(inj.default_on)
    }
}

/// Builds the single initialization script with every enabled module.
pub fn script(config: &Config) -> String {
    let mut out = String::from(
        "(() => {\n\
         if (location.origin !== 'https://music.youtube.com') return;\n\
         const feathertune = {};\n\
         Object.defineProperty(window, '__feathertune', { value: feathertune });\n\
         const addStyle = (id, css) => {\n\
         \x20 const s = document.createElement('style');\n\
         \x20 s.dataset.feathertune = id;\n\
         \x20 s.textContent = css;\n\
         \x20 const add = () => (document.head || document.documentElement).append(s);\n\
         \x20 document.documentElement ? add() : document.addEventListener('readystatechange', add, { once: true });\n\
         };\n",
    );
    for inj in INJECTIONS.iter().filter(|i| config.enabled(i)) {
        let id = serde_json::to_string(inj.id).unwrap();
        out.push_str("try {\n");
        if let Some(css) = inj.css {
            out.push_str(&format!("addStyle({id}, {});\n", serde_json::to_string(css).unwrap()));
        }
        if let Some(js) = inj.js {
            out.push_str(&format!("((feathertune) => {{\n{js}\n}})(feathertune);\n"));
        }
        out.push_str(&format!("}} catch (e) {{ console.error('[feathertune] injection ' + {id} + ' failed', e); }}\n"));
    }
    out.push_str("})();\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_follow_their_switch() {
        assert!(script(&Config::default()).contains("SELECTORS"));
        let off: Config = toml::from_str("[injections]\ncontrols = false").unwrap();
        assert!(!script(&off).contains("SELECTORS"));
    }
}
