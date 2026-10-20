fn main() {
    // Declares the app's only command, so it gets an `allow-player-state` permission and is denied
    // unless a capability grants it (capabilities/player-bridge.json).
    let manifest = tauri_build::AppManifest::new().commands(&["player_state"]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest)).expect("tauri-build failed");
}
