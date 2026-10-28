//! `ext_call`: what the imported extensions' page code (injections/extensions/page.js) asks Rust for, and the
//! same operations for their WebAssembly (wasm.rs). The site's own scripts run in the same JavaScript world as
//! the page code and can call the command too, claiming any id, so it only offers what is fine for the site to
//! have: it never acts on the player or the app, and each operation checks that the id is a running extension
//! that declared the permission it needs (docs/DESIGN.md, Extensions).
//!
//! `op` (page and WebAssembly):
//! - `error` ({message}): shown in the Extensions tab and the diagnostics. `log` ({message}): the diagnostics,
//!   at most 30 lines a minute.
//! - `storage_get` ({key}) → the value or null, `storage_set` ({key, value}), `storage_remove` ({key}),
//!   `storage_keys` → [key]: JSON values in `extension-data/<id>.json`, at most 1 MB (permission `storage`).
//! - `fetch` ({url, method?, headers?, body?}) → {status, headers, body}: https only, to the hosts of its `net:`
//!   permissions, no redirects followed, 10 s, a 1 MB body out and 2 MB back as text, 60 a minute.
//! - `notify` ({title, body?}): a toast under the app's name, one per 10 s (permission `notify`).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tauri::{AppHandle, Manager, Webview};

use crate::extensions::{DATA_DIR, Extension, Extensions};

const MAX_MESSAGE: usize = 2000;
const MAX_STORAGE: usize = 1 << 20;
const MAX_KEY: usize = 128;
const MAX_REQUEST_BODY: usize = 1 << 20;
const MAX_RESPONSE_BODY: u64 = 2 << 20;
const MAX_HEADERS: usize = 20;
/// Headers an extension may not set: the client sets these (or they would carry the user's cookies).
const FORBIDDEN_HEADERS: &[&str] =
    &["host", "cookie", "content-length", "connection", "transfer-encoding", "upgrade", "proxy-authorization"];

#[tauri::command]
pub async fn ext_call(webview: Webview, app: AppHandle, id: String, op: String, arg: Value) -> Result<Value, String> {
    if webview.label() != "site" {
        return Err("not allowed".into());
    }
    // Requests and files are slow next to the page: off the main thread and the async workers.
    tauri::async_runtime::spawn_blocking(move || {
        let ext = app.state::<Extensions>().get(&id).cloned().ok_or("unknown extension")?;
        call(&app, &ext, &op, &arg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The operations the page and the WebAssembly share.
pub fn call(app: &AppHandle, ext: &Extension, op: &str, arg: &Value) -> Result<Value, String> {
    let message = || {
        arg.get("message")
            .and_then(Value::as_str)
            .filter(|m| m.len() <= MAX_MESSAGE)
            .map(|m| m.chars().filter(|c| !c.is_control() || *c == '\n').collect::<String>())
            .ok_or("expected a message")
    };
    let key =
        || arg.get("key").and_then(Value::as_str).filter(|k| (1..=MAX_KEY).contains(&k.len())).ok_or("expected a key");
    match op {
        "error" => {
            app.state::<Extensions>().error(&ext.id, &message()?);
            Ok(Value::Null)
        }
        "log" => {
            if allow(&ext.id, "log", 30, Duration::from_secs(60)) {
                crate::diag::log(&format!("ext {}", ext.id), message()?);
            }
            Ok(Value::Null)
        }
        "storage_get" | "storage_set" | "storage_remove" | "storage_keys" => {
            need(ext, "storage")?;
            let store = app.state::<Storage>();
            let _lock = store.0.lock().unwrap();
            let path = app.state::<crate::settings::Live>().profile.join(DATA_DIR).join(format!("{}.json", ext.id));
            let mut data: Map<String, Value> =
                std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            match op {
                "storage_get" => return Ok(data.get(key()?).cloned().unwrap_or(Value::Null)),
                "storage_keys" => return Ok(json!(data.keys().collect::<Vec<_>>())),
                "storage_set" => {
                    let value = arg.get("value").cloned().ok_or("expected a value")?;
                    data.insert(key()?.to_owned(), value);
                }
                _ => {
                    data.remove(key()?);
                }
            }
            let text = serde_json::to_vec(&data).map_err(|e| e.to_string())?;
            if text.len() > MAX_STORAGE {
                return Err("storage is full (1 MB)".into());
            }
            std::fs::create_dir_all(path.parent().expect("a folder")).map_err(|e| e.to_string())?;
            std::fs::write(&path, text).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "fetch" => {
            if !allow(&ext.id, "fetch", 60, Duration::from_secs(60)) {
                return Err("too many requests (60 a minute)".into());
            }
            fetch(ext, arg)
        }
        "notify" => {
            need(ext, "notify")?;
            let title = arg.get("title").and_then(Value::as_str).filter(|t| !t.is_empty() && t.len() <= 200);
            let body = arg.get("body").and_then(Value::as_str).unwrap_or("");
            let title = title.ok_or("expected a title of up to 200 bytes")?;
            if body.len() > 500 {
                return Err("the body is over 500 bytes".into());
            }
            if !allow(&ext.id, "notify", 1, Duration::from_secs(10)) {
                return Err("one notification per 10 s".into());
            }
            notify(app, title, body)
        }
        _ => Err("unknown operation".into()),
    }
}

/// Serializes the storage files' reads and writes.
#[derive(Default)]
pub struct Storage(Mutex<()>);

fn need(ext: &Extension, permission: &str) -> Result<(), String> {
    if ext.permissions.iter().any(|p| p == permission) {
        Ok(())
    } else {
        Err(format!("the manifest does not ask for {permission}"))
    }
}

/// The times of each extension's recent calls of each kind.
type Calls = HashMap<(String, &'static str), VecDeque<Instant>>;

/// At most `max` calls of `kind` per `per` for one extension.
fn allow(id: &str, kind: &'static str, max: usize, per: Duration) -> bool {
    static CALLS: Mutex<Option<Calls>> = Mutex::new(None);
    let mut calls = CALLS.lock().unwrap();
    let times = calls.get_or_insert_default().entry((id.to_owned(), kind)).or_default();
    let now = Instant::now();
    while times.front().is_some_and(|t| now.duration_since(*t) > per) {
        times.pop_front();
    }
    if times.len() >= max {
        return false;
    }
    times.push_back(now);
    true
}

/// Whether `host` is one of the extension's `net:` permissions (`net:a.com` is that host only, `net:*.a.com`
/// its subdomains and itself).
fn allows_host(ext: &Extension, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ext.permissions.iter().filter_map(|p| p.strip_prefix("net:")).any(|p| match p.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == p,
    })
}

fn fetch(ext: &Extension, arg: &Value) -> Result<Value, String> {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

    let url = arg.get("url").and_then(Value::as_str).ok_or("expected a url")?;
    let parsed: tauri::Url = url.parse().map_err(|_| "invalid url")?;
    if parsed.scheme() != "https" || !parsed.host_str().is_some_and(|h| allows_host(ext, h)) {
        return Err("only https, to a host the manifest declares (net:)".into());
    }
    let method = arg.get("method").and_then(Value::as_str).unwrap_or("GET").to_ascii_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&method.as_str()) {
        return Err("unsupported method".into());
    }
    let mut request = ureq::http::Request::builder().method(method.as_str()).uri(url);
    if let Some(headers) = arg.get("headers").and_then(Value::as_object) {
        if headers.len() > MAX_HEADERS {
            return Err("too many headers".into());
        }
        for (name, value) in headers {
            let value = value.as_str().filter(|v| v.len() <= 1024).ok_or("header values are text of up to 1 KB")?;
            if FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err(format!("the {name} header cannot be set"));
            }
            request = request.header(name.as_str(), value);
        }
    }
    let body = arg.get("body").and_then(Value::as_str).unwrap_or("");
    if body.len() > MAX_REQUEST_BODY {
        return Err("the body is over 1 MB".into());
    }
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION"), " (extension)"))
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(10)))
            .build(),
    );
    let sent = if body.is_empty() {
        agent.run(request.body(()).map_err(|e| e.to_string())?)
    } else {
        agent.run(request.body(body.to_owned()).map_err(|e| e.to_string())?)
    };
    let mut response = sent.map_err(|e| format!("request failed: {e}"))?;
    let headers: Map<String, Value> = response
        .headers()
        .iter()
        .filter(|(name, _)| !name.as_str().eq_ignore_ascii_case("set-cookie"))
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), json!(value.to_str().ok()?))))
        .collect();
    let status = response.status().as_u16();
    let bytes = if method == "HEAD" {
        Vec::new()
    } else {
        response.body_mut().with_config().limit(MAX_RESPONSE_BODY).read_to_vec().map_err(|e| e.to_string())?
    };
    Ok(json!({ "status": status, "headers": headers, "body": String::from_utf8_lossy(&bytes) }))
}

fn notify(app: &AppHandle, title: &str, body: &str) -> Result<Value, String> {
    use tauri_winrt_notification::Toast;
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};

    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let mut toast = Toast::new(&app.config().identifier).title(title).sound(None);
    if !body.is_empty() {
        toast = toast.text1(body);
    }
    toast.show().map_err(|e| e.to_string())?;
    Ok(Value::Null)
}
