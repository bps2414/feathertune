//! Global shortcuts (tauri-plugin-global-shortcut, plugin `global_shortcuts`): play/pause, next,
//! previous and the mini player, with the keys from `[shortcuts]` in `config.toml`. They run the same
//! commands as the tray menu. `register` can run again at any time (the settings window does).

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::settings::Live;

/// Replaces every registered shortcut with the current settings.
pub fn register(app: &AppHandle) {
    let shortcuts = app.global_shortcut();
    if let Err(e) = shortcuts.unregister_all() {
        eprintln!("global shortcuts not cleared: {e}");
    }
    let live = app.state::<Live>();
    if !live.plugin("global_shortcuts") {
        return;
    }
    let wanted = {
        let s = &live.get().shortcuts;
        [
            (s.play_pause.clone(), "playPause"),
            (s.next.clone(), "next"),
            (s.previous.clone(), "previous"),
            (s.mini_player.clone(), "miniPlayer"),
        ]
    };
    for (keys, command) in wanted {
        let keys = keys.trim();
        if keys.is_empty() {
            continue;
        }
        let result = shortcuts.on_shortcut(keys, move |app, _, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            if command == "miniPlayer" {
                crate::miniplayer::toggle(app);
            } else {
                crate::player(app, command);
            }
        });
        if let Err(e) = result {
            eprintln!("global shortcut {keys:?} not registered: {e}");
        }
    }
}
