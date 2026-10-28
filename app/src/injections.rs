//! Page injections: every piece of JS and CSS that enters the page lives in `injections/<id>/`
//! (`page.js` and/or `style.css`) and is compiled into the binary.
//!
//! Each module except `core` (the player state bridge, always on) belongs to one or more plugins
//! (plugins.rs) and is loaded when any of their switches in `config.toml` is on. Every plugin is on by
//! default; the switch is the control.
//! On startup the enabled modules are assembled into a single initialization script that only
//! runs on the main frame of https://music.youtube.com. CSS enters as a `<style>` element and each
//! module runs inside its own `try/catch`, so a module that throws does not take the others down.
//! Modules share the `feathertune` namespace object (`window.__feathertune`), which is how Rust reaches them.
//!
//! Rust gives a module data with `send(app, module, &json)`: it calls `window.__feathertune.<module>.set(data)`
//! with the data parsed from a JSON string, so the page only ever receives data, never code built from
//! strings, and the page asks for nothing. The last data of each module is kept and sent again when the
//! page finishes loading (a sign-in or a reload loses the page's copy); the same data twice in a row is
//! sent once.
//!
//! To add a module: create its folder, add one entry to `INJECTIONS` and its plugins to `PLUGINS`.
//!
//! Imported extensions (extensions.rs) are not modules: the `extensions` host goes at the end of this script and
//! each extension is an initialization script of its own; their settings reach the page as the `ext` module's data.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Manager, Webview};

use crate::extensions::Extension;
use crate::settings::Settings;

struct Injection {
    id: &'static str,
    js: Option<&'static str>,
    css: Option<&'static str>,
    /// The plugins whose switches turn the module on (any of them); empty: always on.
    plugins: &'static [&'static str],
}

const INJECTIONS: &[Injection] = &[
    Injection { id: "core", js: Some(include_str!("../injections/core/page.js")), css: None, plugins: &[] },
    Injection {
        id: "controls",
        js: Some(include_str!("../injections/controls/page.js")),
        css: None,
        plugins: &["controls"],
    },
    Injection {
        id: "theme",
        js: Some(include_str!("../injections/theme/page.js")),
        css: Some(include_str!("../injections/theme/style.css")),
        plugins: &["album_color"],
    },
    Injection {
        id: "lyrics",
        js: Some(include_str!("../injections/lyrics/page.js")),
        css: Some(include_str!("../injections/lyrics/style.css")),
        plugins: &["synced_lyrics"],
    },
    Injection {
        id: "ambient",
        js: Some(include_str!("../injections/ambient/page.js")),
        css: Some(include_str!("../injections/ambient/style.css")),
        plugins: &["ambient_mode"],
    },
    Injection {
        id: "song_mode",
        js: Some(include_str!("../injections/song_mode/page.js")),
        css: None,
        plugins: &["song_mode"],
    },
    Injection {
        id: "precise_volume",
        js: Some(include_str!("../injections/precise_volume/page.js")),
        css: Some(include_str!("../injections/precise_volume/style.css")),
        plugins: &["precise_volume"],
    },
    Injection {
        id: "skip_disliked",
        js: Some(include_str!("../injections/skip_disliked/page.js")),
        css: None,
        plugins: &["skip_disliked"],
    },
    Injection {
        id: "audio",
        js: Some(include_str!("../injections/audio/page.js")),
        css: Some(include_str!("../injections/audio/style.css")),
        plugins: crate::audio::PLUGINS,
    },
];

/// The last data `send` gave each module (as JSON text).
#[derive(Default)]
pub struct PageData(Mutex<BTreeMap<&'static str, String>>);

/// Gives `module` (a `feathertune.<module>` object with a `set` function) its data. `Value::Null` clears.
/// The same data as last time is not sent again (the page has it; a `set` would only redo its work).
/// Callable from any thread.
pub fn send(app: &AppHandle, module: &'static str, data: &Value) {
    let json = data.to_string();
    if app.state::<PageData>().0.lock().unwrap().insert(module, json.clone()).as_ref() == Some(&json) {
        return;
    }
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
    // The extensions' page code reads the lyrics and the album colors too (`ft.lyrics`, `ft.theme`).
    if matches!(module, "lyrics" | "theme")
        && site.state::<crate::extensions::Extensions>().running().iter().any(|e| e.js.is_some())
    {
        let _ = site.eval(format!("window.__feathertune?.ext?.feed('{module}', JSON.parse({literal}))"));
    }
}

/// The host of the imported extensions: loaded when any running one has page code (extensions.rs).
const EXTENSIONS_HOST: &str = include_str!("../injections/extensions/page.js");

/// Builds the single initialization script with every enabled module, and the extensions' host when `extensions`
/// (the running ones) has page code. The extensions themselves are scripts of their own (`extensions::page_scripts`).
pub fn script(settings: &Settings, extensions: &[Arc<Extension>]) -> String {
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
    for inj in INJECTIONS.iter().filter(|i| i.plugins.is_empty() || i.plugins.iter().any(|p| settings.plugin(p))) {
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
    let expected: Vec<&str> =
        extensions.iter().filter(|e| e.js.is_some() || e.css.is_some()).map(|e| e.id.as_str()).collect();
    if !expected.is_empty() {
        let expected = serde_json::to_string(&expected).unwrap();
        out.push_str(&format!(
            "try {{\n((feathertune) => {{\nconst EXPECTED = {expected};\n{EXTENSIONS_HOST}\n}})(feathertune);\n\
             }} catch (e) {{ console.error('[feathertune] extensions host failed', e); }}\n"
        ));
    }
    out.push_str("})();\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modules_follow_their_switch() {
        // A selector only `controls` uses.
        assert!(script(&Settings::default(), &[]).contains(".previous-button"));
        let off = Settings::parse("[plugins]\ncontrols = false").unwrap();
        assert!(!script(&off, &[]).contains(".previous-button"));
        assert!(script(&off, &[]).contains("player_state"));
    }

    #[test]
    fn every_module_plugin_exists() {
        for inj in INJECTIONS {
            for p in inj.plugins {
                assert!(crate::plugins::get(p).is_some(), "{} {p}", inj.id);
            }
        }
    }
}
