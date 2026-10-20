//! Global shortcuts (tauri-plugin-global-shortcut): play/pause, next and previous, with the keys from
//! `[shortcuts]` in `config.toml`. They run the same `controls` commands as the tray menu.

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::Shortcuts;

pub fn register(app: &AppHandle, shortcuts: &Shortcuts) {
    let wanted = [(&shortcuts.play_pause, "playPause"), (&shortcuts.next, "next"), (&shortcuts.previous, "previous")];
    for (keys, command) in wanted {
        let keys = keys.trim();
        if keys.is_empty() {
            continue;
        }
        let result = app.global_shortcut().on_shortcut(keys, move |app, _, event| {
            if event.state == ShortcutState::Pressed {
                crate::player(app, command);
            }
        });
        if let Err(e) = result {
            eprintln!("global shortcut {keys:?} not registered: {e}");
        }
    }
}
