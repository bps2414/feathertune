//! `ext_call`: what the imported extensions' page code (injections/extensions/page.js) asks Rust for. The site's
//! own scripts run in the same JavaScript world and can call it too, claiming any id, so it only offers what is
//! fine for the site to have: it never acts on the player or the app, and each operation checks that the id is a
//! running extension that declared the permission it needs (docs/DESIGN.md, Extensions).
//!
//! `op`: `error` ({message}: shown in the Extensions tab and the diagnostics).

use serde_json::Value;
use tauri::{AppHandle, Manager, Webview};

use crate::extensions::Extensions;

const MAX_MESSAGE: usize = 2000;

#[tauri::command]
pub fn ext_call(webview: Webview, app: AppHandle, id: String, op: String, arg: Value) -> Result<Value, String> {
    if webview.label() != "site" {
        return Err("not allowed".into());
    }
    let extensions = app.state::<Extensions>();
    let ext = extensions.get(&id).ok_or("unknown extension")?;
    let message =
        || arg.get("message").and_then(Value::as_str).filter(|m| m.len() <= MAX_MESSAGE).ok_or("expected a message");
    match op.as_str() {
        "error" => {
            extensions.error(&ext.id, message()?);
            Ok(Value::Null)
        }
        _ => Err("unknown operation".into()),
    }
}
