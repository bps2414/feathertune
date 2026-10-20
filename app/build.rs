fn main() {
    // Declares the app's commands, so each gets an `allow-<command>` permission and is denied unless a
    // capability grants it: `player_state` to the site webview (capabilities/player-bridge.json), the
    // `settings_*` commands to the settings window only (capabilities/settings-window.json).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "player_state",
        "settings_get",
        "settings_set",
        "settings_action",
        "settings_stats",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("tauri-build failed");
}
