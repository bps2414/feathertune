//! Page injections: every piece of JS and CSS that enters the page lives in `injections/<id>/`
//! (`page.js` and/or `style.css`) and is compiled into the binary.
//!
//! Each module except `core` (the player state bridge, always on) belongs to a plugin (plugins.rs)
//! and follows its switch in `config.toml`. A module that is an unmeasured experiment gets a plugin
//! with `default_on: false` until a measurement shows a gain.
//! On startup the enabled modules are assembled into a single initialization script that only
//! runs on the main frame of https://music.youtube.com. CSS enters as a `<style>` element and each
//! module runs inside its own `try/catch`, so a module that throws does not take the others down.
//! Modules share the `feathertune` namespace object (`window.__feathertune`), which is how Rust reaches them.
//!
//! To add a module: create its folder, add one entry to `INJECTIONS` and its plugin to `PLUGINS`.

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
];

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
