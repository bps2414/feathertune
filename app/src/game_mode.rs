//! Game mode: the title bar button with a game controller. On, it turns off at once the plugins whose
//! cost was measured clearly above the noise (docs/DESIGN.md, Plugins), and a second click brings them
//! back as they were. Only in memory: `config.toml` and the settings' switches are not touched, and it
//! ends with the app.
//!
//! - `synced_lyrics`: the page gets no lines (the site's own lyrics come back and the Lyrics tab's
//!   scroll loop stops); the lookup goes on for the floating lyrics and the mini player.
//! - `ambient_mode`: the page module takes its layers and its style out.
//! - `desktop_lyrics`: closed, and opened again afterwards if it was open.
//! - `share_lyrics`: its window is closed (the button still opens it).
//!
//! Left alone: everything measured inside the noise, `animated_title_bar` (+3.8 points against a noise
//! of 2.8: about the noise) and what was not measured yet.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tauri::AppHandle;

static ON: AtomicBool = AtomicBool::new(false);

pub fn is_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Turns game mode on or off. Any thread.
pub fn toggle(app: &AppHandle) {
    let on = !ON.fetch_xor(true, Ordering::Relaxed);
    crate::diag::log("game mode", if on { "on" } else { "off" });
    crate::lyrics::set_page_paused(app, on);
    crate::injections::send(app, "ambient", &json!({ "off": on }));
    crate::desktop_lyrics::set_suspended(app, on);
    if on {
        crate::share_lyrics::close(app);
    }
    crate::titlebar::refresh(app);
}
