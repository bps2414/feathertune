//! A feathertune extension's `plugin.wasm` in Rust: the host ABI of docs/EXTENSIONS.md (WebAssembly) behind a
//! few functions. Build a `cdylib` for `wasm32-unknown-unknown` and name your event handler with [`main!`]:
//!
//! ```ignore
//! feathertune_ext::main!(on_event);
//!
//! fn on_event(event: feathertune_ext::Event) {
//!     if event["type"] == "track" {
//!         feathertune_ext::log(&format!("now playing {}", event["state"]["title"]));
//!     }
//! }
//! ```

pub use serde_json::{Value, json};

/// One event as the app sends it: `{"type": "init" | "track" | "state" | "ad" | "settings" | "page" | "timer", ...}`.
pub type Event = Value;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "ft")]
unsafe extern "C" {
    #[link_name = "call"]
    fn ft_call(ptr: *const u8, len: usize) -> i32;
    #[link_name = "take"]
    fn ft_take(ptr: *mut u8, cap: usize) -> i32;
    #[link_name = "log"]
    fn ft_log(ptr: *const u8, len: usize);
}

/// Sends a request (`{"op": ..., ...}`) to the app and returns its answer.
#[cfg(target_arch = "wasm32")]
pub fn request(request: &Value) -> Result<Value, String> {
    let bytes = request.to_string();
    let len = unsafe { ft_call(bytes.as_ptr(), bytes.len()) };
    let mut answer = vec![0u8; len.max(0) as usize];
    let n = unsafe { ft_take(answer.as_mut_ptr(), answer.len()) };
    answer.truncate(n.max(0) as usize);
    let mut answer: Value = serde_json::from_slice(&answer).map_err(|e| e.to_string())?;
    match answer.get_mut("error") {
        Some(Value::String(e)) => Err(std::mem::take(e)),
        _ => Ok(answer.get_mut("ok").map(Value::take).unwrap_or(Value::Null)),
    }
}

/// Off WebAssembly (tests, `cargo check` on the host) there is no app to ask.
#[cfg(not(target_arch = "wasm32"))]
pub fn request(_request: &Value) -> Result<Value, String> {
    Err("not running in feathertune".into())
}

/// `op` with the fields of `args` (an object, or null).
pub fn call(op: &str, args: Value) -> Result<Value, String> {
    let mut request = match args {
        Value::Object(map) => map,
        _ => Default::default(),
    };
    request.insert("op".into(), op.into());
    self::request(&Value::Object(request))
}

/// A line in the app's diagnostics (at most 30 a minute).
pub fn log(text: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        ft_log(text.as_ptr(), text.len())
    };
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{text}");
}

/// The player state of now (title, artist, position, paused...), or null.
pub fn state() -> Result<Value, String> {
    call("state", Value::Null)
}

/// The play queue.
pub fn queue() -> Result<Value, String> {
    call("queue", Value::Null)
}

/// The extension's settings of now.
pub fn settings() -> Result<Value, String> {
    call("settings", Value::Null)
}

/// Milliseconds since 1970 (the module has no clock of its own).
pub fn now() -> Result<u64, String> {
    call("now", Value::Null).map(|v| v.as_u64().unwrap_or(0))
}

/// play, pause, playPause, next, previous; `seek` (seconds) and `volume` (0 to 100) take a value.
/// Needs the `player.control` permission.
pub fn command(name: &str, value: Option<f64>) -> Result<Value, String> {
    call("command", json!({ "name": name, "value": value }))
}

/// A `timer` event with this `id` after `ms` milliseconds (10 ms to 24 h, at most 16 waiting).
pub fn timer(ms: u64, id: Value) -> Result<(), String> {
    call("timer", json!({ "ms": ms, "id": id })).map(drop)
}

/// A message to the extension's own page code (`ft.on('native', ...)`), at most 256 KB.
pub fn page_send(data: Value) -> Result<(), String> {
    call("page_send", json!({ "data": data })).map(drop)
}

/// The extension's storage (permission `storage`): JSON values, 1 MB in all.
pub mod storage {
    use super::{Value, call, json};

    pub fn get(key: &str) -> Result<Value, String> {
        call("storage_get", json!({ "key": key }))
    }

    pub fn set(key: &str, value: Value) -> Result<(), String> {
        call("storage_set", json!({ "key": key, "value": value })).map(drop)
    }

    pub fn remove(key: &str) -> Result<(), String> {
        call("storage_remove", json!({ "key": key })).map(drop)
    }

    pub fn keys() -> Result<Vec<String>, String> {
        let keys = call("storage_keys", Value::Null)?;
        Ok(keys
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_str().map(String::from))
            .collect())
    }
}

/// An https request to a host of the manifest's `net:` permissions: `{status, headers, body}`.
pub fn fetch(url: &str, method: &str, headers: Value, body: &str) -> Result<Value, String> {
    call(
        "fetch",
        json!({ "url": url, "method": method, "headers": headers, "body": body }),
    )
}

/// A Windows notification (permission `notify`, one per 10 s).
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    call("notify", json!({ "title": title, "body": body })).map(drop)
}

/// Shown in the Extensions tab, under the extension.
pub fn error(text: &str) {
    let _ = call("error", json!({ "message": text }));
}

/// Allocates `len` bytes for the app to write an event into (the `ft_alloc` export).
#[doc(hidden)]
pub fn __alloc(len: i32) -> i32 {
    let block = vec![0u8; len.max(0) as usize].into_boxed_slice();
    Box::into_raw(block) as *mut u8 as usize as i32
}

/// Reads the event at `ptr`, frees it and hands it to `handler` (the `ft_event` export).
#[doc(hidden)]
pub fn __event(ptr: i32, len: i32, handler: fn(Event)) {
    let bytes = unsafe {
        Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr as usize as *mut u8,
            len.max(0) as usize,
        ))
    };
    match serde_json::from_slice(&bytes) {
        Ok(event) => handler(event),
        Err(e) => error(&format!("bad event: {e}")),
    }
}

/// Exports `ft_alloc` and `ft_event`, giving every event to `handler: fn(Event)`.
#[macro_export]
macro_rules! main {
    ($handler:path) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn ft_alloc(len: i32) -> i32 {
            $crate::__alloc(len)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn ft_event(ptr: i32, len: i32) {
            $crate::__event(ptr, len, $handler)
        }
    };
}
