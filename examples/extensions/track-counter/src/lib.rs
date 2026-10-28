//! Counts the tracks played, keeps the count in the extension's storage, shows a notification every
//! `every` tracks and answers its page code with the count.

use feathertune_ext::{Event, Value, json, log, notify, page_send, storage};

feathertune_ext::main!(on_event);

fn count() -> u64 {
    storage::get("count")
        .ok()
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn on_event(event: Event) {
    match event["type"].as_str() {
        Some("init") => log(&format!(
            "track counter {} started at {}",
            event["version"].as_str().unwrap_or(""),
            count()
        )),
        Some("track") => {
            let n = count() + 1;
            if let Err(e) = storage::set("count", json!(n)) {
                feathertune_ext::error(&e);
            }
            let every = feathertune_ext::settings()
                .ok()
                .and_then(|s| s["every"].as_u64())
                .unwrap_or(100);
            if every > 0 && n.is_multiple_of(every) {
                let _ = notify(
                    &format!("{n} tracks"),
                    event["state"]["title"].as_str().unwrap_or(""),
                );
            }
            let _ = page_send(json!({ "count": n }));
        }
        Some("page") if event["data"]["get"] == Value::Bool(true) => {
            let _ = page_send(json!({ "count": count() }));
        }
        _ => {}
    }
}
