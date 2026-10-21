//! The `audio` page module (`injections/audio/page.js`): one Web Audio graph on the page's own
//! `<video>`, shared by every audio plugin. It is loaded when any of `PLUGINS` is on at startup, and
//! the graph is only created on the first `play`. This side gives the page its data (which audio
//! plugins are on and the `[audio]` settings), live; the page never asks for anything here.
//!
//! Rules (docs/DESIGN.md, Audio): the sound is processed and measured on the page only, for the
//! listener; every effect is bypassed while an ad plays.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};
use tauri::{AppHandle, Manager};

use crate::settings::{Live, Settings};

/// The plugins of the `audio` page module.
pub const PLUGINS: &[&str] = &["equalizer", "compressor"];

/// `--measure`: the graph's state, printed every 10 s.
pub const PROBE: &str = "(() => window.__feathertune?.audio?.probe?.() ?? 'audio=none')()";

/// The module is in the page (some audio plugin was on at startup).
static LOADED: AtomicBool = AtomicBool::new(false);

pub fn start(app: &AppHandle) {
    let on = PLUGINS.iter().any(|p| app.state::<Live>().plugin(p));
    LOADED.store(on, Ordering::Relaxed);
    if on {
        push(app);
    }
}

/// Whether the page has the module; without it, turning an audio plugin on needs a restart.
pub fn loaded() -> bool {
    LOADED.load(Ordering::Relaxed)
}

/// Gives the page the current settings (after a change in the settings window).
pub fn push(app: &AppHandle) {
    if !loaded() {
        return;
    }
    let data = payload(&app.state::<Live>().get());
    crate::injections::send(app, "audio", &data);
}

fn payload(s: &Settings) -> Value {
    let plugins: Map<String, Value> = PLUGINS.iter().map(|p| (p.to_string(), json!(s.plugin(p)))).collect();
    json!({ "plugins": plugins })
}
