//! Page injections: every piece of JS and CSS that enters the page lives in `injections/<id>/`
//! (`page.js` and/or `style.css`) and is compiled into the binary.
//!
//! Each module except `core` (the player state bridge, always on) belongs to a plugin (plugins.rs)
//! and follows its switch in `config.toml`. Every plugin is on by default; the switch is the control.
//! On startup the enabled modules are assembled into a single initialization script that only
//! runs on the main frame of https://music.youtube.com. CSS enters as a `<style>` element and each
//! module runs inside its own `try/catch`, so a module that throws does not take the others down.
//! Modules share the `feathertune` namespace object (`window.__feathertune`), which is how Rust reaches them.
//!
//! Rust gives a module data with `send(app, module, &json)`: it calls `window.__feathertune.<module>.set(data)`
//! with the data parsed from a JSON string, so the page only ever receives data, never code built from
//! strings, and the page asks for nothing. The last data of each module is kept and sent again when the
//! page finishes loading (a sign-in or a reload loses the page's copy).
//!
//! To add a module: create its folder, add one entry to `INJECTIONS` and its plugin to `PLUGINS`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;
use tauri::{AppHandle, Manager, Webview};

use crate::settings::Settings;

struct Injection {
    id: &'static str,
    js: Option<&'static str>,
    css: Option<&'static str>,
    /// The plugin whose switch turns the module on; `None`: always on.
    plugin: Option<&'static str>,
}

const INJECTIONS: &[Injection] = &[
    Injection { id: "core", js: Some(include_str!("../injections/core/page.js")), css: None, plugin: None },
    Injection {
        id: "controls",
        js: Some(include_str!("../injections/controls/page.js")),
        css: None,
        plugin: Some("controls"),
    },
    Injection {
        id: "theme",
        js: Some(include_str!("../injections/theme/page.js")),
        css: Some(include_str!("../injections/theme/style.css")),
        plugin: Some("album_color"),
    },
    Injection {
        id: "lyrics",
        js: Some(include_str!("../injections/lyrics/page.js")),
        css: Some(include_str!("../injections/lyrics/style.css")),
        plugin: Some("synced_lyrics"),
    },
    Injection {
        id: "ambient",
        js: Some(include_str!("../injections/ambient/page.js")),
        css: Some(include_str!("../injections/ambient/style.css")),
        plugin: Some("ambient_mode"),
    },
];

/// The last data `send` gave each module (as JSON text).
#[derive(Default)]
pub struct PageData(Mutex<BTreeMap<&'static str, String>>);

/// Gives `module` (a `feathertune.<module>` object with a `set` function) its data. `Value::Null` clears.
/// Callable from any thread.
pub fn send(app: &AppHandle, module: &'static str, data: &Value) {
    let json = data.to_string();
    app.state::<PageData>().0.lock().unwrap().insert(module, json.clone());
    if let Some(site) = app.get_webview("site") {
        push(&site, module, &json);
    }
}

/// Sends every module its last data again (the page finished loading).
pub fn resend(site: &Webview) {
    let all: Vec<_> = site.state::<PageData>().0.lock().unwrap().iter().map(|(m, j)| (*m, j.clone())).collect();
    for (module, json) in all {
        push(site, module, &json);
    }
}

fn push(site: &Webview, module: &str, json: &str) {
    // `module` is one of our constant ids; the data goes in as a string literal that the page parses.
    let literal = serde_json::to_string(json).expect("a string serializes");
    let _ = site.eval(format!("window.__feathertune?.{module}?.set(JSON.parse({literal}))"));
}

/// Builds the single initialization script with every enabled module.
pub fn script(settings: &Settings) -> String {
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
    for inj in INJECTIONS.iter().filter(|i| i.plugin.is_none_or(|p| settings.plugin(p))) {
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
        assert!(script(&Settings::default()).contains("SELECTORS"));
        let off = Settings::parse("[plugins]\ncontrols = false").unwrap();
        assert!(!script(&off).contains("SELECTORS"));
        assert!(script(&off).contains("player_state"));
    }

    #[test]
    fn every_module_plugin_exists() {
        for inj in INJECTIONS {
            assert!(inj.plugin.is_none_or(|p| crate::plugins::get(p).is_some()), "{}", inj.id);
        }
    }
}
