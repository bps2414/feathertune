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
//!
//! Page only: `native` ({data}): a message to the extension's own WebAssembly, at most 256 KB (wasm.rs).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tauri::{AppHandle, Manager, Webview};

use crate::extensions::{DATA_DIR, Extension, Extensions};

const MAX_ARG: usize = 2 << 20;
const MAX_FETCH_IN_FLIGHT: usize = 8;
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
pub async fn ext_call(webview: Webview, app: AppHandle, id: String, op: String, arg: String) -> Result<Value, String> {
    if webview.label() != "site" {
        return Err("not allowed".into());
    }
    if arg.len() > MAX_ARG {
        return Err("argument is over 2 MB".into());
    }
    // Requests and files are slow next to the page: off the main thread and the async workers.
    tauri::async_runtime::spawn_blocking(move || {
        let ext = app.state::<Extensions>().get(&id).cloned().ok_or("unknown extension")?;
        let arg: Value = serde_json::from_str(&arg).map_err(|e| e.to_string())?;
        if op == "native" {
            let data = arg.get("data").ok_or("expected data")?;
            if data.to_string().len() > crate::wasm::MAX_PAGE_MESSAGE {
                return Err("the message is over 256 KB".into());
            }
            return crate::wasm::page_message(&app, &ext.id, data.clone()).map(|()| Value::Null);
        }
        call(&app, &ext, &op, &arg)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The operations the page and the WebAssembly share.
pub fn call(app: &AppHandle, ext: &Extension, op: &str, arg: &Value) -> Result<Value, String> {
    let message = |allow_nl: bool| {
        arg.get("message")
            .and_then(Value::as_str)
            .filter(|m| m.len() <= MAX_MESSAGE)
            .map(|m| m.chars().filter(|c| !c.is_control() || (allow_nl && *c == '\n')).collect::<String>())
            .ok_or("expected a message")
    };
    let key =
        || arg.get("key").and_then(Value::as_str).filter(|k| (1..=MAX_KEY).contains(&k.len())).ok_or("expected a key");
    match op {
        "error" => {
            if allow(&ext.id, "error", 20, Duration::from_secs(60)) {
                app.state::<Extensions>().error(&ext.id, &message(true)?);
            }
            Ok(Value::Null)
        }
        "log" => {
            if allow(&ext.id, "log", 30, Duration::from_secs(60)) {
                crate::diag::log(&format!("ext {}", ext.id), message(false)?);
            }
            Ok(Value::Null)
        }
        "storage_get" | "storage_set" | "storage_remove" | "storage_keys" => {
            need(ext, "storage")?;
            let store = app.state::<Storage>();
            let _lock = store.0.lock().unwrap();
            let path = app.state::<crate::settings::Live>().profile.join(DATA_DIR).join(format!("{}.json", ext.id));
            let mut data: Map<String, Value> = match std::fs::read(&path) {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("storage is unreadable: {e}"))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
                Err(e) => return Err(format!("could not read storage: {e}")),
            };
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
            crate::settings::write_atomic(&path, &text).map_err(|e| e.to_string())?;
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
fn host_matches(permissions: &[String], host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    permissions.iter().filter_map(|p| p.strip_prefix("net:")).any(|p| match p.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => host == p,
    })
}

fn allows_host(ext: &Extension, host: &str) -> bool {
    host_matches(&ext.permissions, host)
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !ip.is_unspecified()
        && o[0] != 0
        && !ip.is_loopback()
        && !ip.is_private()
        && !ip.is_link_local()
        && !ip.is_multicast()
        && !ip.is_broadcast()
        && !ip.is_documentation()
        && !(o[0] == 100 && (64..=127).contains(&o[1]))
        && !(o[0] == 192 && o[1] == 0 && o[2] == 0)
        && !(o[0] == 198 && (18..=19).contains(&o[1]))
        && o[0] < 240
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ipv4(v4);
            }
            let segs = v6.segments();
            if segs[..6] == [0; 6] && !(segs[6] == 0 && segs[7] <= 1) {
                return is_public_ipv4(Ipv4Addr::from(((segs[6] as u32) << 16) | (segs[7] as u32)));
            }
            !v6.is_unspecified()
                && !v6.is_loopback()
                && !v6.is_multicast()
                && (segs[0] & 0xfe00) != 0xfc00
                && (segs[0] & 0xffc0) != 0xfe80
                && (segs[0] & 0xffc0) != 0xfec0
                && !(segs[0] == 0x2001 && segs[1] == 0x0db8)
        }
    }
}

#[derive(Debug)]
struct PublicResolver(ureq::unversioned::resolver::DefaultResolver);

impl ureq::unversioned::resolver::Resolver for PublicResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let resolved = self.0.resolve(uri, config, timeout)?;
        let mut out = self.empty();
        for &addr in &resolved {
            if is_public_ip(addr.ip()) {
                out.push(addr);
            }
        }
        if out.is_empty() { Err(ureq::Error::HostNotFound) } else { Ok(out) }
    }
}

fn fetch_agent() -> &'static ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    use ureq::unversioned::resolver::DefaultResolver;
    use ureq::unversioned::transport::DefaultConnector;

    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let config = ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION"), " (extension)"))
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(10)))
            .build();
        ureq::Agent::with_parts(config, DefaultConnector::default(), PublicResolver(DefaultResolver::default()))
    })
}

struct FlightGuard;
impl Drop for FlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    }
}
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

fn acquire_flight() -> Result<FlightGuard, String> {
    let mut cur = IN_FLIGHT.load(Ordering::Relaxed);
    loop {
        if cur >= MAX_FETCH_IN_FLIGHT {
            return Err("too many concurrent requests".into());
        }
        match IN_FLIGHT.compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Ok(FlightGuard),
            Err(next) => cur = next,
        }
    }
}

fn fetch(ext: &Extension, arg: &Value) -> Result<Value, String> {
    let url = arg.get("url").and_then(Value::as_str).ok_or("expected a url")?;
    let parsed: tauri::Url = url.parse().map_err(|_| "invalid url")?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || !parsed.host_str().is_some_and(|h| allows_host(ext, h))
    {
        return Err("only https, to a host the manifest declares (net:)".into());
    }
    let method = arg.get("method").and_then(Value::as_str).unwrap_or("GET").to_ascii_uppercase();
    if !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(&method.as_str()) {
        return Err("unsupported method".into());
    }
    let mut request = ureq::http::Request::builder().method(method.as_str()).uri(parsed.as_str());
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
    let _flight = acquire_flight()?;
    let agent = fetch_agent();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_ip_blocks_private_loopback_cgnat_and_mapped() {
        for bad in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "::ffff:127.0.0.1",
            "::ffff:100.64.0.1",
            "::ffff:169.254.169.254",
        ] {
            let ip: IpAddr = bad.parse().unwrap();
            assert!(!is_public_ip(ip), "{bad} must be rejected");
        }
        for good in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:4700:4700::1111", "::ffff:1.1.1.1"] {
            let ip: IpAddr = good.parse().unwrap();
            assert!(is_public_ip(ip), "{good} should be public");
        }
    }

    #[test]
    fn host_matching_enforces_domain_boundaries() {
        let perms = vec!["net:api.example.com".to_owned(), "net:*.cdn.example.org".to_owned()];
        assert!(host_matches(&perms, "API.Example.Com"));
        assert!(!host_matches(&perms, "sub.api.example.com"));
        assert!(host_matches(&perms, "cdn.example.org"));
        assert!(host_matches(&perms, "a.b.cdn.example.org"));
        assert!(!host_matches(&perms, "evilcdn.example.org"));
    }
}
