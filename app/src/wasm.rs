//! The imported extensions' native code: `plugin.wasm`, run by `wasmi` (an interpreter, no JIT) on a thread of
//! its own, with no WASI. It only reaches the app through three host functions, so it touches no file, socket or
//! window by itself (docs/EXTENSIONS.md, WebAssembly):
//!
//! - `ft.call(ptr, len) -> len`: a JSON request `{op, ...}`; the answer, `{"ok": value}` or `{"error": text}`,
//!   waits in the host and its length comes back. `ft.take(ptr, cap) -> len` copies it into the module's memory.
//!   `ft.log(ptr, len)`: a text line in the diagnostics.
//! - The module exports `memory`, `ft_alloc(len) -> ptr` and `ft_event(ptr, len)`, which gets each event as JSON:
//!   `init` (id, version, settings, the app's version), `track`, `state` (play, pause, seek, volume), `ad`,
//!   `settings`, `page` (a message of its own page code) and `timer`.
//!
//! `op`: `state`, `queue`, `settings`, `now`, `command` ({name, value?}: play, pause, playPause, next, previous,
//! seek, volume; permission `player.control`; the same functions the tray calls, refused by the page during an
//! ad), `page_send` ({data}: to its own page code, `ft.on('native')`), `timer` ({ms, id?}: a `timer` event later,
//! at most 16 waiting), and every operation of `extension_api::call` (storage, fetch, notify, log, error).
//!
//! Limits: 64 MB of memory, 2·10⁹ units of fuel for the start and for each event (a few seconds); running out, a
//! trap or a bad pointer stops the extension until the next start and shows the error in the Extensions tab.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use wasmi::{Caller, Config, Engine, Error, Extern, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};

use crate::extensions::{Extension, Extensions};
use crate::player::{Event, Player, State};

const MAX_MEMORY: usize = 64 << 20;
const FUEL: u64 = 2_000_000_000;
const MAX_REQUEST: usize = 1 << 20;
const MAX_TIMERS: usize = 16;
const QUEUE: usize = 256;
pub const MAX_PAGE_MESSAGE: usize = 256 << 10;

/// The running modules' event queues, by extension id.
#[derive(Default)]
pub struct Wasm(Mutex<HashMap<String, SyncSender<Value>>>);

impl Wasm {
    fn send(&self, id: &str, event: Value) {
        let mut all = self.0.lock().unwrap();
        if let Some(tx) = all.get(id)
            && let Err(TrySendError::Disconnected(_)) = tx.try_send(event)
        {
            all.remove(id);
        }
    }

    fn broadcast(&self, event: &Value) {
        let ids: Vec<String> = self.0.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.send(&id, event.clone());
        }
    }
}

/// Starts every running extension that has a `plugin.wasm`, and the thread that gives them the player's events.
pub fn start(app: &AppHandle) {
    let running: Vec<Arc<Extension>> = app.state::<Extensions>().running().iter().filter(|e| e.wasm).cloned().collect();
    if running.is_empty() {
        return;
    }
    for ext in running {
        let (tx, rx) = sync_channel(QUEUE);
        app.state::<Wasm>().0.lock().unwrap().insert(ext.id.clone(), tx.clone());
        let app = app.clone();
        let _ = std::thread::Builder::new().name(format!("wasm {}", ext.id)).spawn(move || {
            if let Err(e) = run(&app, &ext, tx, rx) {
                app.state::<Extensions>().error(&ext.id, &format!("WebAssembly stopped: {e}"));
            }
            app.state::<Wasm>().0.lock().unwrap().remove(&ext.id);
        });
    }
    let mut rx = app.state::<Player>().subscribe();
    let app = app.clone();
    std::thread::spawn(move || {
        let mut ad = false;
        loop {
            let event = match rx.blocking_recv() {
                Ok(Event::Track(s)) => json!({ "type": "track", "state": state_json(&s) }),
                Ok(Event::Playback(s)) if s.ad != ad => {
                    ad = s.ad;
                    json!({ "type": "ad", "ad": ad, "state": state_json(&s) })
                }
                Ok(Event::Playback(s)) => json!({ "type": "state", "state": state_json(&s) }),
                Ok(Event::Progress(_)) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            app.state::<Wasm>().broadcast(&event);
        }
    });
}

/// A message of an extension's page code to its own module (`ext_call` `native`).
pub fn page_message(app: &AppHandle, id: &str, data: Value) -> Result<(), String> {
    let wasm = app.state::<Wasm>();
    if !wasm.0.lock().unwrap().contains_key(id) {
        return Err("no WebAssembly running for this extension".into());
    }
    wasm.send(id, json!({ "type": "page", "data": data }));
    Ok(())
}

/// Gives the running modules their settings of now.
pub fn settings(app: &AppHandle) {
    let live = app.state::<crate::settings::Live>();
    let settings = live.get();
    for ext in app.state::<Extensions>().running().iter().filter(|e| e.wasm) {
        app.state::<Wasm>().send(&ext.id, json!({ "type": "settings", "settings": ext.values(&settings) }));
    }
}

fn state_json(s: &State) -> Value {
    json!({
        "id": s.track.id,
        "title": s.track.title,
        "artist": s.track.artist,
        "album": s.track.album,
        "artwork": s.track.artwork,
        "duration": s.track.duration,
        "position": (s.position_now() * 10.0).round() / 10.0,
        "paused": s.paused,
        "volume": s.volume,
        "muted": s.muted,
        "ad": s.ad,
    })
}

struct Host {
    app: AppHandle,
    ext: Arc<Extension>,
    /// The last answer of `ft.call`, until `ft.take` copies it.
    answer: Vec<u8>,
    limits: StoreLimits,
    tx: SyncSender<Value>,
    timers: Arc<AtomicUsize>,
}

fn guest_bytes(caller: &Caller<'_, Host>, ptr: i32, len: i32) -> Result<Vec<u8>, Error> {
    let len = usize::try_from(len).map_err(|_| Error::new("negative length"))?;
    if len > MAX_REQUEST {
        return Err(Error::new("a request over 1 MB"));
    }
    let memory = caller.get_export("memory").and_then(Extern::into_memory).ok_or(Error::new("no memory export"))?;
    let mut buf = vec![0; len];
    memory.read(caller, ptr as u32 as usize, &mut buf).map_err(|e| Error::new(e.to_string()))?;
    Ok(buf)
}

fn run(app: &AppHandle, ext: &Arc<Extension>, tx: SyncSender<Value>, rx: Receiver<Value>) -> Result<(), String> {
    let path = app.state::<Extensions>().dir().join(&ext.id).join("plugin.wasm");
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &bytes).map_err(|e| e.to_string())?;
    let host = Host {
        app: app.clone(),
        ext: ext.clone(),
        answer: Vec::new(),
        limits: StoreLimitsBuilder::new().memory_size(MAX_MEMORY).instances(1).tables(4).memories(1).build(),
        tx,
        timers: Arc::default(),
    };
    let mut store = Store::new(&engine, host);
    store.limiter(|h| &mut h.limits);
    let mut linker = <Linker<Host>>::new(&engine);
    linker
        .func_wrap("ft", "log", |caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<(), Error> {
            let text = String::from_utf8_lossy(&guest_bytes(&caller, ptr, len)?).into_owned();
            let host = caller.data();
            let _ = crate::extension_api::call(&host.app, &host.ext, "log", &json!({ "message": text }));
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("ft", "call", |mut caller: Caller<'_, Host>, ptr: i32, len: i32| -> Result<i32, Error> {
            let request = guest_bytes(&caller, ptr, len)?;
            let answer = match serde_json::from_slice::<Value>(&request) {
                Ok(request) => match host_call(caller.data(), &request) {
                    Ok(value) => json!({ "ok": value }),
                    Err(e) => json!({ "error": e }),
                },
                Err(e) => json!({ "error": format!("not JSON: {e}") }),
            };
            let answer = answer.to_string().into_bytes();
            let len = i32::try_from(answer.len()).map_err(|_| Error::new("answer too large"))?;
            caller.data_mut().answer = answer;
            Ok(len)
        })
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("ft", "take", |mut caller: Caller<'_, Host>, ptr: i32, cap: i32| -> Result<i32, Error> {
            let answer = std::mem::take(&mut caller.data_mut().answer);
            let n = answer.len().min(usize::try_from(cap).unwrap_or(0));
            let memory =
                caller.get_export("memory").and_then(Extern::into_memory).ok_or(Error::new("no memory export"))?;
            memory.write(&mut caller, ptr as u32 as usize, &answer[..n]).map_err(|e| Error::new(e.to_string()))?;
            Ok(n as i32)
        })
        .map_err(|e| e.to_string())?;

    store.set_fuel(FUEL).map_err(|e| e.to_string())?;
    let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| e.to_string())?;
    let alloc = instance.get_typed_func::<i32, i32>(&store, "ft_alloc").map_err(|e| format!("ft_alloc: {e}"))?;
    let on_event =
        instance.get_typed_func::<(i32, i32), ()>(&store, "ft_event").map_err(|e| format!("ft_event: {e}"))?;
    let memory = instance.get_memory(&store, "memory").ok_or("no memory export")?;

    let settings = ext.values(&app.state::<crate::settings::Live>().get());
    let mut event = json!({
        "type": "init",
        "id": ext.id,
        "version": ext.version,
        "settings": settings,
        "app_version": env!("CARGO_PKG_VERSION"),
    });
    loop {
        let bytes = event.to_string().into_bytes();
        let len = i32::try_from(bytes.len()).map_err(|_| "event too large")?;
        store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let ptr = alloc.call(&mut store, len).map_err(|e| e.to_string())?;
        memory.write(&mut store, ptr as u32 as usize, &bytes).map_err(|e| e.to_string())?;
        on_event.call(&mut store, (ptr, len)).map_err(|e| e.to_string())?;
        if event["type"] == "timer" {
            store.data().timers.fetch_sub(1, Ordering::Relaxed);
        }
        event = rx.recv().map_err(|_| "closed")?;
    }
}

/// One `ft.call` request.
fn host_call(host: &Host, request: &Value) -> Result<Value, String> {
    let op = request.get("op").and_then(Value::as_str).ok_or("expected op")?;
    let app = &host.app;
    let ext = &host.ext;
    match op {
        "state" => Ok(app.state::<Player>().current().map_or(Value::Null, |s| state_json(&s))),
        "queue" => serde_json::to_value(app.state::<Player>().queue().items).map_err(|e| e.to_string()),
        "settings" => Ok(Value::Object(ext.values(&app.state::<crate::settings::Live>().get()))),
        "now" => Ok(json!(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))),
        "command" => {
            if !ext.permissions.iter().any(|p| p == "player.control") {
                return Err("the manifest does not ask for player.control".into());
            }
            let value = request.get("value").and_then(Value::as_f64);
            let sent = match request.get("name").and_then(Value::as_str) {
                Some(name @ ("play" | "pause" | "playPause" | "next" | "previous")) => {
                    crate::player(app, name);
                    true
                }
                Some("seek") => crate::player_seek(app, value.ok_or("expected a value in seconds")?),
                Some("volume") => crate::player_set_volume(app, value.ok_or("expected a value from 0 to 100")?),
                _ => return Err("unknown command".into()),
            };
            Ok(json!(sent))
        }
        "page_send" => {
            let data = request.get("data").ok_or("expected data")?;
            let json = data.to_string();
            if json.len() > MAX_PAGE_MESSAGE {
                return Err("the message is over 256 KB".into());
            }
            let site = app.get_webview("site").ok_or("no page")?;
            let id = serde_json::to_string(&ext.id).expect("a string serializes");
            let literal = serde_json::to_string(&json).expect("a string serializes");
            site.eval(format!("window.__feathertune?.ext?.native({id}, JSON.parse({literal}))"))
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "timer" => {
            let ms = request.get("ms").and_then(Value::as_u64).filter(|ms| (10..=86_400_000).contains(ms));
            let ms = ms.ok_or("ms from 10 to 86400000")?;
            if host.timers.fetch_add(1, Ordering::Relaxed) >= MAX_TIMERS {
                host.timers.fetch_sub(1, Ordering::Relaxed);
                return Err(format!("at most {MAX_TIMERS} timers waiting"));
            }
            let tx = host.tx.clone();
            let event = json!({ "type": "timer", "id": request.get("id").cloned().unwrap_or(Value::Null) });
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                let _ = tx.send(event);
            });
            Ok(Value::Null)
        }
        _ => crate::extension_api::call(app, ext, op, request),
    }
}
