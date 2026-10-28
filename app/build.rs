fn main() {
    // Declares the app's commands, so each gets an `allow-<command>` permission and is denied unless a
    // capability grants it: `player_state`, `audio_facts` and `mix_open` to the site webview
    // (capabilities/player-bridge.json), the `settings_*` commands to the settings window only
    // (capabilities/settings-window.json).
    let manifest = tauri_build::AppManifest::new().commands(&[
        "player_state",
        "audio_facts",
        "mix_open",
        "ext_call",
        "settings_get",
        "settings_set",
        "settings_action",
        "settings_stats",
        "settings_mix",
        "settings_remote",
        "settings_extensions",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("tauri-build failed");
}
