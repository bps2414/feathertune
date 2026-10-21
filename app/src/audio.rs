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
pub const PLUGINS: &[&str] = &["equalizer", "compressor", "crossfade"];

/// Equalizer presets: gains in dB for 31, 62, 125, 250, 500 Hz, 1, 2, 4, 8 and 16 kHz ("custom" is
/// `[audio] eq_bands`).
pub const EQ_PRESETS: &[(&str, [f64; 10])] = &[
    ("flat", [0.0; 10]),
    ("bass", [6.0, 5.0, 4.0, 2.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("vocal", [-2.0, -2.0, -1.0, 0.0, 1.5, 3.0, 3.0, 2.0, 0.0, -1.0]),
    ("treble", [0.0, 0.0, 0.0, 0.0, 0.0, 0.5, 1.5, 3.5, 5.0, 6.0]),
    ("loudness", [5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 0.0, 1.0, 3.0, 4.0]),
];
pub const EQ_MAX_DB: f64 = 12.0;

/// Compressor presets (their settings are in the page module); "none" leaves it out of the path.
pub const COMPRESSORS: &[&str] = &["none", "soft", "night", "strong"];

/// Crossfade curves (built in the page module).
pub const CURVES: &[&str] = &["linear", "equal_power", "s_curve", "exponential"];
pub const MAX_CROSSFADE_MS: u32 = 12_000;
pub const MAX_FADE_IN_MS: u32 = 5_000;

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

/// The equalizer gains the settings ask for (flat for anything unknown).
pub fn eq_gains(s: &Settings) -> [f64; 10] {
    if s.audio.eq_preset == "custom" {
        let mut gains = [0.0; 10];
        for (g, v) in gains.iter_mut().zip(&s.audio.eq_bands) {
            *g = if v.is_finite() { v.clamp(-EQ_MAX_DB, EQ_MAX_DB) } else { 0.0 };
        }
        return gains;
    }
    EQ_PRESETS.iter().find(|(name, _)| *name == s.audio.eq_preset).map_or([0.0; 10], |(_, g)| *g)
}

fn payload(s: &Settings) -> Value {
    let plugins: Map<String, Value> = PLUGINS.iter().map(|p| (p.to_string(), json!(s.plugin(p)))).collect();
    let compressor =
        if COMPRESSORS.contains(&s.audio.compressor.as_str()) { s.audio.compressor.as_str() } else { "none" };
    let a = &s.audio;
    let crossfade = s.plugin("crossfade").then(|| {
        json!({
            "duration_ms": a.crossfade_ms.min(MAX_CROSSFADE_MS),
            "curve": if CURVES.contains(&a.curve.as_str()) { a.curve.as_str() } else { "equal_power" },
            "fade_in_ms": a.fade_in_ms.min(MAX_FADE_IN_MS),
            "echo": a.echo_out,
        })
    });
    json!({ "plugins": plugins, "eq": eq_gains(s), "compressor": compressor, "crossfade": crossfade })
}
