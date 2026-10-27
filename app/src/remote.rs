//! Phone remote (plugin `remote`, Experimental): a WebSocket server on the local network that shows paired
//! phones what plays and takes their commands. Plan, security model and the wire protocol: docs/REMOTE.md.
//!
//! - Listens on `[remote] port` (IPv4, and IPv6 when it can) only while the plugin is on; `apply` follows the
//!   switch, and turning it off closes the port and every connection. A source outside the private,
//!   link-local, loopback and CGNAT ranges is dropped before the WebSocket handshake, as is a sixth try from
//!   one address within a minute. At most 4 sessions at once.
//! - One binary WebSocket frame is one Noise message. `/v1/pair`: `Noise_IKpsk2` with the QR's one-time secret
//!   as the PSK, then the user allows the phone after comparing a 6-digit code. `/v1`: `Noise_KK` with a paired
//!   phone's key; an unknown key is closed after its first message, and nothing is sent before the phone's
//!   first `ping` (which a replayed handshake cannot produce).
//! - The PC's static key is in the Windows credential store (keyring); the paired phones (name, public key,
//!   when) are in `remote-devices.json` in the profile.
//! - Sessions get `hello`, then `state` (on every `player::Event`), `queue` (on every queue change) and
//!   `library` (the library's playlists, on every change). Commands go to the `controls` injection through
//!   main.rs's player functions; all but play and pause are refused while an ad plays.
//! - The managed `Remote` is the API for the settings window: `start_pairing`, `stop_pairing`, `pairing`,
//!   `pending`, `answer`, `devices`, `remove`, `forget_all`, `listening` and `on_change`.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::future::{Either, select};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use snow::TransportState;
use tauri::async_runtime::{self, JoinHandle};
use tauri::{AppHandle, Manager};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout, timeout_at};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{self, Message, http};

use crate::player::{Event, Player, Playlist, Queue, State};
use crate::settings::Live;

pub const DEFAULT_PORT: u16 = 47652;
pub const DEFAULT_NAME: &str = "feathertune";
/// Protocol version (`hello`'s `v`) and the oldest one this PC still speaks (`min`).
/// 2 added the queue actions, `open_playlist` and `library`; a version 1 phone just never sends them.
const VERSION: u32 = 2;
const MIN_VERSION: u32 = 1;
/// The pairing QR's own format (its `v`), which version 2 did not change.
const QR_VERSION: u32 = 1;
const PAIR_PATTERN: &str = "Noise_IKpsk2_25519_ChaChaPoly_SHA256";
const KK_PATTERN: &str = "Noise_KK_25519_ChaChaPoly_SHA256";
const PAIR_PROLOGUE: &[u8] = b"feathertune-remote/1 pair";
const KK_PROLOGUE: &[u8] = b"feathertune-remote/1";
const DEVICES_FILE: &str = "remote-devices.json";
const KEYRING_SERVICE: &str = "feathertune";
const KEYRING_USER: &str = "remote-key";

const PAIRING_WINDOW: Duration = Duration::from_secs(120);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The controller pings every 15 s; three missed and the session is closed.
const SILENCE: Duration = Duration::from_secs(45);
const MINUTE: Duration = Duration::from_secs(60);
const HANDSHAKES_PER_MINUTE: usize = 5;
const MAX_SESSIONS: usize = 4;
/// Paired phones; also bounds the keys a `KK` knock is tried against.
const MAX_DEVICES: usize = 16;
const MAX_NAME: usize = 32;
/// The largest Noise message, and the AEAD tag each one carries.
const NOISE_MAX: usize = 65_535;
const TAG: usize = 16;
/// The largest plaintext from a controller, and the frame that carries it (handshake frames are smaller).
const MAX_IN: usize = 1024;
const MAX_FRAME: usize = MAX_IN + TAG;
/// The largest plaintext from the PC (one Noise message).
const MAX_OUT: usize = NOISE_MAX - TAG;
/// Messages waiting for one session; a slow one misses states (the next one is whole again).
const SESSION_QUEUE: usize = 64;
/// Queue rows `play_item` may name, and the longest seek (like player.rs).
const MAX_QUEUE: usize = 200;
const MAX_SECONDS: f64 = 86_400.0;

/// Why the PC closes a connection: the WebSocket close code and reason the phone reads (docs/REMOTE.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Close {
    Denied,
    Expired,
    Unknown,
    Removed,
    Busy,
    Timeout,
    Invalid,
    TooBig,
    Off,
    Full,
    /// The phone left: nothing to send.
    Gone,
}

impl Close {
    fn frame(self) -> Option<(u16, &'static str)> {
        Some(match self {
            Close::Denied => (4000, "denied"),
            Close::Expired => (4001, "expired"),
            Close::Unknown => (4002, "unknown"),
            Close::Removed => (4003, "removed"),
            Close::Busy => (4004, "busy"),
            Close::Timeout => (4005, "timeout"),
            Close::Invalid => (4006, "invalid"),
            Close::TooBig => (4007, "too_big"),
            Close::Off => (4008, "off"),
            Close::Full => (4009, "full"),
            Close::Gone => return None,
        })
    }
}

/// What the server reads from and drives: the app in production, a fake in the tests.
trait Host: Send + Sync + 'static {
    fn state(&self) -> Option<State>;
    fn queue(&self) -> Queue;
    fn subscribe(&self) -> broadcast::Receiver<Event>;
    fn subscribe_queue(&self) -> broadcast::Receiver<Queue>;
    fn library(&self) -> Vec<Playlist>;
    fn subscribe_library(&self) -> broadcast::Receiver<Vec<Playlist>>;
    fn colors(&self) -> Vec<String>;
    /// Sends a validated command to the page; false when there was no page to send it to.
    fn run(&self, command: &Command) -> bool;
}

struct AppHost(AppHandle);

impl Host for AppHost {
    fn state(&self) -> Option<State> {
        self.0.state::<Player>().current()
    }

    fn queue(&self) -> Queue {
        self.0.state::<Player>().queue()
    }

    fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.0.state::<Player>().subscribe()
    }

    fn subscribe_queue(&self) -> broadcast::Receiver<Queue> {
        self.0.state::<Player>().subscribe_queue()
    }

    fn library(&self) -> Vec<Playlist> {
        self.0.state::<Player>().library()
    }

    fn subscribe_library(&self) -> broadcast::Receiver<Vec<Playlist>> {
        self.0.state::<Player>().subscribe_library()
    }

    fn colors(&self) -> Vec<String> {
        crate::album_color::colors()
    }

    fn run(&self, command: &Command) -> bool {
        let app = &self.0;
        match command {
            Command::Play {} => crate::player_call(app, "play()"),
            Command::Pause {} => crate::player_call(app, "pause()"),
            Command::Next {} => crate::player_call(app, "next()"),
            Command::Previous {} => crate::player_call(app, "previous()"),
            Command::Seek { s } => crate::player_seek(app, *s),
            Command::Volume { v } => crate::player_set_volume(app, *v),
            Command::PlayItem { id, i } => crate::player_play_item(app, id, *i),
            Command::QueueNext { id, i } => crate::player_queue_next(app, id, *i),
            Command::QueueRemove { id, i } => crate::player_queue_remove(app, id, *i),
            Command::QueueMove { id, i, to } => crate::player_queue_move(app, id, *i, *to),
            Command::OpenPlaylist { list } => crate::player_open_playlist(app, list),
        }
    }
}

/// A controller message, parsed into a fixed shape.
#[derive(Deserialize, Debug)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
enum Incoming {
    Cmd(Command),
    Ping { n: u64 },
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(tag = "c", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Play {},
    Pause {},
    Next {},
    Previous {},
    Seek { s: f64 },
    Volume { v: f64 },
    PlayItem { id: String, i: usize },
    QueueNext { id: String, i: usize },
    QueueRemove { id: String, i: usize },
    QueueMove { id: String, i: usize, to: usize },
    OpenPlaylist { list: String },
}

impl Command {
    fn valid(&self) -> bool {
        let row = |id: &str, i: usize| crate::player::valid_id(id) && i < MAX_QUEUE;
        match self {
            Command::Seek { s } => s.is_finite() && (0.0..=MAX_SECONDS).contains(s),
            Command::Volume { v } => v.is_finite() && (0.0..=100.0).contains(v),
            Command::PlayItem { id, i } | Command::QueueNext { id, i } | Command::QueueRemove { id, i } => row(id, *i),
            Command::QueueMove { id, i, to } => row(id, *i) && *to < MAX_QUEUE && to != i,
            Command::OpenPlaylist { list } => crate::player::valid_list(list),
            _ => true,
        }
    }

    /// Pausing or resuming an ad shortens nothing; everything else waits for it to end.
    fn refused_during_ad(&self) -> bool {
        !matches!(self, Command::Play {} | Command::Pause {})
    }
}

/// The payload of the first pairing message.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairHello {
    name: String,
}

/// The phone's first transport message on `/v1/pair`: it proves the phone had the secret.
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
enum Confirm {
    Pair {},
}

#[derive(Clone)]
struct Keys {
    private: [u8; 32],
    public: [u8; 32],
}

impl Keys {
    fn generate() -> Keys {
        let pair = snow::Builder::new(params(KK_PATTERN)).generate_keypair().expect("X25519 key pair");
        Keys {
            private: pair.private.try_into().expect("32-byte key"),
            public: pair.public.try_into().expect("32-byte key"),
        }
    }

    /// Both halves in hex, as kept in the credential store.
    fn to_text(&self) -> String {
        hex(&self.private) + &hex(&self.public)
    }

    fn from_text(text: &str) -> Option<Keys> {
        (text.len() == 128 && text.is_ascii())
            .then(|| Some(Keys { private: unhex32(&text[..64])?, public: unhex32(&text[64..])? }))?
    }
}

/// A paired phone, as stored in `remote-devices.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Device {
    name: String,
    /// The phone's static public key, lowercase hex.
    key: String,
    /// Unix seconds.
    paired: u64,
}

/// A paired phone, for the settings window.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub key: String,
    pub name: String,
    pub paired: u64,
    /// Unix seconds of its last session (this run only).
    pub last_seen: Option<u64>,
    pub connected: bool,
}

/// A phone that proved it saw the QR and waits for Allow or Deny.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pending {
    pub name: String,
    /// The 6 digits the phone shows too.
    pub code: String,
}

struct Pairing {
    /// Taken by the first pairing message that reads (made with the PC's key), whatever comes of it.
    secret: Option<[u8; 32]>,
    until: Instant,
    asking: Option<Asking>,
}

struct Asking {
    name: String,
    code: String,
    answer: oneshot::Sender<bool>,
}

enum Out {
    Json(Value),
    Close(Close),
}

struct Session {
    key: String,
    tx: mpsc::Sender<Out>,
}

struct Running {
    port: u16,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Default)]
struct Inner {
    keys: Option<Keys>,
    name: String,
    devices: Vec<Device>,
    last_seen: HashMap<String, u64>,
    pairing: Option<Pairing>,
    sessions: HashMap<u64, Session>,
    next_session: u64,
    attempts: HashMap<IpAddr, Vec<Instant>>,
    running: Option<Running>,
}

type OnChange = Arc<dyn Fn() + Send + Sync>;

pub struct Shared {
    host: Arc<dyn Host>,
    devices_file: PathBuf,
    /// The PC key is kept in the credential store (the tests keep it in memory).
    keyring: bool,
    inner: Mutex<Inner>,
    on_change: Mutex<Option<OnChange>>,
}

/// The managed remote (see the module comment).
pub struct Remote(Arc<Shared>);

impl std::ops::Deref for Remote {
    type Target = Shared;

    fn deref(&self) -> &Shared {
        &self.0
    }
}

impl Remote {
    pub fn new(app: &AppHandle, profile: &Path) -> Remote {
        Remote(Arc::new(Shared::new(Arc::new(AppHost(app.clone())), profile.join(DEVICES_FILE), true)))
    }
}

/// Starts, moves (a new port) or stops the listener to match the settings.
pub fn apply(app: &AppHandle) {
    let Some(remote) = app.try_state::<Remote>() else { return };
    let (on, port, name) = {
        let live = app.state::<Live>();
        let s = live.get();
        (s.plugin("remote"), s.remote.port, s.remote.name.clone())
    };
    let port = if port >= 1024 { port } else { DEFAULT_PORT };
    let running = {
        let mut inner = remote.lock();
        inner.name = valid_name(&name).unwrap_or_else(|| DEFAULT_NAME.into());
        inner.running.as_ref().map(|r| r.port)
    };
    if !on {
        if running.is_some() {
            remote.stop();
            println!("remote: stopped");
        }
        return;
    }
    if running == Some(port) {
        return;
    }
    let addrs = [SocketAddr::from(([0, 0, 0, 0], port)), SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))];
    match remote.0.listen(&addrs) {
        Ok(port) => println!("remote: listening on port {port}"),
        Err(e) => eprintln!("remote: {e}"),
    }
}

/// The album colors changed: the sessions get the state again with them.
pub fn refresh(app: &AppHandle) {
    if let Some(remote) = app.try_state::<Remote>()
        && let Some(state) = state_json(&*remote.host)
    {
        remote.broadcast(state);
    }
}

impl Shared {
    fn new(host: Arc<dyn Host>, devices_file: PathBuf, keyring: bool) -> Shared {
        let devices = load_devices(&devices_file);
        Shared {
            host,
            devices_file,
            keyring,
            inner: Mutex::new(Inner { devices, name: DEFAULT_NAME.into(), ..Default::default() }),
            on_change: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// Called (from any thread) whenever the pairing window, the pending request, the paired list or a
    /// connection changes.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Arc::new(f));
    }

    /// Called without the lock: the callback may ask for the state.
    fn changed(&self) {
        let f = self.on_change.lock().unwrap().clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Opens a pairing window of 2 minutes and returns the QR's text (docs/REMOTE.md, Protocol). A new call
    /// replaces the window and its secret.
    pub fn start_pairing(&self) -> Result<String, String> {
        let addrs = lan_addresses();
        if addrs.is_empty() {
            return Err("no local network address found".into());
        }
        let (port, key, secret) = self.new_pairing()?;
        Ok(pairing_uri(&addrs, port, &key, &secret))
    }

    /// The port, the PC's public key and a fresh secret of a new pairing window.
    fn new_pairing(&self) -> Result<(u16, [u8; 32], [u8; 32]), String> {
        let secret = random32()?;
        let mut inner = self.lock();
        let port = inner.running.as_ref().ok_or("the remote is off")?.port;
        let key = inner.keys.as_ref().ok_or("the remote is off")?.public;
        inner.pairing = Some(Pairing { secret: Some(secret), until: Instant::now() + PAIRING_WINDOW, asking: None });
        drop(inner);
        self.changed();
        Ok((port, key, secret))
    }

    /// Ends the pairing window (the settings screen closed); a phone waiting for an answer is refused.
    pub fn stop_pairing(&self) {
        self.lock().pairing = None;
        self.changed();
    }

    /// The open pairing window: the seconds left, and whether its QR is still good (a phone that scanned it
    /// has used the secret).
    pub fn pairing(&self) -> Option<(u64, bool)> {
        let inner = self.lock();
        let pairing = inner.pairing.as_ref()?;
        let left = pairing.until.checked_duration_since(Instant::now())?;
        Some((left.as_secs_f64().ceil() as u64, pairing.secret.is_some()))
    }

    /// The port while listening; `None` when the plugin is off or the port could not be opened.
    pub fn listening(&self) -> Option<u16> {
        self.lock().running.as_ref().map(|r| r.port)
    }

    pub fn pending(&self) -> Option<Pending> {
        let inner = self.lock();
        let pairing = inner.pairing.as_ref().filter(|p| p.until > Instant::now())?;
        pairing.asking.as_ref().map(|a| Pending { name: a.name.clone(), code: a.code.clone() })
    }

    /// Allow (store the phone and start its session) or deny the pending phone. False: none was pending.
    pub fn answer(&self, allow: bool) -> bool {
        let asking = self.lock().pairing.as_mut().and_then(|p| p.asking.take());
        asking.is_some_and(|a| a.answer.send(allow).is_ok())
    }

    pub fn devices(&self) -> Vec<DeviceInfo> {
        let inner = self.lock();
        inner
            .devices
            .iter()
            .map(|d| DeviceInfo {
                key: d.key.clone(),
                name: d.name.clone(),
                paired: d.paired,
                last_seen: inner.last_seen.get(&d.key).copied(),
                connected: inner.sessions.values().any(|s| s.key == d.key),
            })
            .collect()
    }

    /// Forgets one phone and closes its session at once. False: no such phone.
    pub fn remove(&self, key: &str) -> bool {
        let found = {
            let mut inner = self.lock();
            let before = inner.devices.len();
            inner.devices.retain(|d| d.key != key);
            let found = inner.devices.len() != before;
            if found {
                self.save(&inner.devices);
            }
            found
        };
        self.kick(|k| k == key, Close::Removed);
        self.changed();
        found
    }

    /// Forgets every phone and makes a new PC key, so no old pairing (or a copy of it) works again.
    pub fn forget_all(&self) -> Result<(), String> {
        let keys = Keys::generate();
        if self.keyring {
            store_keys(&keys)?;
        }
        {
            let mut inner = self.lock();
            inner.keys = Some(keys);
            inner.devices.clear();
            inner.last_seen.clear();
            inner.pairing = None;
            self.save(&inner.devices);
        }
        self.kick(|_| true, Close::Removed);
        self.changed();
        Ok(())
    }

    fn save(&self, devices: &[Device]) {
        let text = serde_json::to_string_pretty(devices).expect("devices serialize");
        if let Err(e) = std::fs::write(&self.devices_file, text) {
            eprintln!("remote: could not save the paired devices: {e}");
        }
    }

    fn ensure_keys(&self) -> Result<(), String> {
        if self.lock().keys.is_none() {
            let keys = if self.keyring { load_keys()? } else { Keys::generate() };
            self.lock().keys = Some(keys);
        }
        Ok(())
    }

    /// Listens on `addrs` (the first is required, the others are tried) and returns the port.
    fn listen(self: &Arc<Self>, addrs: &[SocketAddr]) -> Result<u16, String> {
        self.stop();
        self.ensure_keys()?;
        let mut listeners = Vec::new();
        for (i, addr) in addrs.iter().enumerate() {
            match std::net::TcpListener::bind(addr).and_then(|l| l.set_nonblocking(true).map(|_| l)) {
                Ok(l) => listeners.push(l),
                Err(e) if i == 0 => return Err(format!("could not listen on port {}: {e}", addr.port())),
                Err(e) => eprintln!("remote: not listening on {addr}: {e}"),
            }
        }
        let port = listeners[0].local_addr().map_err(|e| e.to_string())?.port();
        let (events, queues, library) =
            (self.host.subscribe(), self.host.subscribe_queue(), self.host.subscribe_library());
        let mut tasks: Vec<JoinHandle<()>> =
            listeners.into_iter().map(|l| async_runtime::spawn(accept(self.clone(), l))).collect();
        tasks.push(async_runtime::spawn(forward_states(self.clone(), events)));
        tasks.push(async_runtime::spawn(forward_queues(self.clone(), queues)));
        tasks.push(async_runtime::spawn(forward_library(self.clone(), library)));
        self.lock().running = Some(Running { port, tasks });
        Ok(port)
    }

    /// Closes the port, every session and the pairing window.
    fn stop(&self) {
        let running = {
            let mut inner = self.lock();
            inner.pairing = None;
            inner.running.take()
        };
        if let Some(running) = running {
            for task in running.tasks {
                task.abort();
            }
            self.kick(|_| true, Close::Off);
            self.changed();
        }
    }

    /// Closes the matching sessions; they leave the list at once.
    fn kick(&self, which: impl Fn(&str) -> bool, why: Close) {
        let mut inner = self.lock();
        inner.sessions.retain(|_, s| {
            if !which(&s.key) {
                return true;
            }
            let _ = s.tx.try_send(Out::Close(why));
            false
        });
    }

    fn broadcast(&self, message: Value) {
        for s in self.lock().sessions.values() {
            let _ = s.tx.try_send(Out::Json(message.clone()));
        }
    }

    /// Whether `ip` may try a handshake now: at most `HANDSHAKES_PER_MINUTE` a minute each.
    fn admit(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        inner.attempts.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < MINUTE);
            !times.is_empty()
        });
        let times = inner.attempts.entry(ip).or_default();
        if times.len() >= HANDSHAKES_PER_MINUTE {
            return false;
        }
        times.push(now);
        true
    }

    /// `/v1/pair`: `IKpsk2`, the phone's proof, then the user's answer. Returns the transport and the phone key.
    async fn pair(&self, ws: &mut Ws) -> Result<(TransportState, String, Option<u64>), Close> {
        let first = read_frame(ws, Instant::now() + HANDSHAKE_TIMEOUT).await?;
        let (secret, until, keys) = {
            let inner = self.lock();
            let keys = inner.keys.clone().ok_or(Close::Off)?;
            let pairing = inner.pairing.as_ref().filter(|p| p.until > Instant::now()).ok_or(Close::Expired)?;
            let secret = pairing.secret.ok_or(Close::Expired)?;
            if inner.devices.len() >= MAX_DEVICES {
                return Err(Close::Full);
            }
            (secret, pairing.until, keys)
        };
        // The QR on the settings screen is spent from here on.
        self.changed();
        let mut hs = snow::Builder::new(params(PAIR_PATTERN))
            .local_private_key(&keys.private)
            .and_then(|b| b.psk(2, &secret))
            .and_then(|b| b.prologue(PAIR_PROLOGUE))
            .and_then(|b| b.build_responder())
            .map_err(|_| Close::Invalid)?;
        let mut buf = vec![0u8; NOISE_MAX];
        // Bytes that do not read as a first message for this PC (anyone on the network can send them) leave the
        // window open; one that reads was made with the PC's key, from the QR.
        let n = hs.read_message(&first, &mut buf).map_err(|_| Close::Invalid)?;
        // One try per secret: it is gone from here on, whatever happens next.
        self.lock()
            .pairing
            .as_mut()
            .filter(|p| p.until == until)
            .and_then(|p| p.secret.take())
            .ok_or(Close::Expired)?;
        let hello: PairHello = serde_json::from_slice(&buf[..n]).map_err(|_| Close::Invalid)?;
        let name = valid_name(&hello.name).ok_or(Close::Invalid)?;
        let key = hs.get_remote_static().map(hex).ok_or(Close::Invalid)?;
        let n = hs.write_message(&[], &mut buf).map_err(|_| Close::Invalid)?;
        send_raw(ws, &buf[..n]).await?;
        let code = pairing_code(hs.get_handshake_hash());
        let mut transport = hs.into_transport_mode().map_err(|_| Close::Invalid)?;
        // The PSK is only mixed in by the PC's reply: the phone's first transport message is what proves it had
        // the secret, so only then is the user asked.
        let frame = read_frame(ws, until).await?;
        let n = transport.read_message(&frame, &mut buf).map_err(|_| Close::Invalid)?;
        serde_json::from_slice::<Confirm>(&buf[..n]).map_err(|_| Close::Invalid)?;
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.lock();
            let pairing = inner.pairing.as_mut().filter(|p| p.until == until).ok_or(Close::Expired)?;
            pairing.asking = Some(Asking { name: name.clone(), code, answer: tx });
        }
        self.changed();
        let result = match timeout_at(until, rx).await {
            Ok(Ok(true)) => {
                let mut inner = self.lock();
                inner.devices.retain(|d| d.key != key);
                inner.devices.push(Device { name, key: key.clone(), paired: now_s() });
                self.save(&inner.devices);
                Ok((transport, key, None))
            }
            Ok(Ok(false)) => Err(Close::Denied),
            _ => Err(Close::Expired),
        };
        // The window ends with its one pairing, whatever the answer.
        {
            let mut inner = self.lock();
            if inner.pairing.as_ref().is_some_and(|p| p.until == until) {
                inner.pairing = None;
            }
        }
        self.changed();
        result
    }

    /// `/v1`: `KK` against each paired key (the PC cannot tell which phone knocks; only its key reads the
    /// first message). Returns the transport, the phone key and its first ping's `n`.
    async fn knock(&self, ws: &mut Ws) -> Result<(TransportState, String, Option<u64>), Close> {
        let first = read_frame(ws, Instant::now() + HANDSHAKE_TIMEOUT).await?;
        let (keys, paired) = {
            let inner = self.lock();
            (inner.keys.clone().ok_or(Close::Off)?, inner.devices.iter().map(|d| d.key.clone()).collect::<Vec<_>>())
        };
        let mut buf = vec![0u8; NOISE_MAX];
        for key in paired {
            let Some(remote) = unhex32(&key) else { continue };
            let Ok(mut hs) = snow::Builder::new(params(KK_PATTERN))
                .local_private_key(&keys.private)
                .and_then(|b| b.remote_public_key(&remote))
                .and_then(|b| b.prologue(KK_PROLOGUE))
                .and_then(|b| b.build_responder())
            else {
                continue;
            };
            if hs.read_message(&first, &mut buf).is_err() {
                continue;
            }
            let n = hs.write_message(&[], &mut buf).map_err(|_| Close::Invalid)?;
            send_raw(ws, &buf[..n]).await?;
            let mut transport = hs.into_transport_mode().map_err(|_| Close::Invalid)?;
            // A replayed first message gets this far; only the phone itself can send a ping now, and nothing is
            // sent or counted before it.
            let frame = read_frame(ws, Instant::now() + HANDSHAKE_TIMEOUT).await?;
            let n = transport.read_message(&frame, &mut buf).map_err(|_| Close::Invalid)?;
            let Ok(Incoming::Ping { n: ping }) = serde_json::from_slice(&buf[..n]) else { return Err(Close::Invalid) };
            return Ok((transport, key, Some(ping)));
        }
        Err(Close::Unknown)
    }

    /// Joins the sessions and queues `hello`, `state` and `queue`, in that order, ahead of any broadcast.
    fn register(&self, key: &str, tx: mpsc::Sender<Out>) -> Result<u64, Close> {
        let mut inner = self.lock();
        if inner.running.is_none() {
            return Err(Close::Off);
        }
        if !inner.devices.iter().any(|d| d.key == key) {
            return Err(Close::Removed);
        }
        // The same phone again (it reconnected before its old socket timed out): the new session wins.
        inner.sessions.retain(|_, s| {
            let same = s.key == key;
            if same {
                let _ = s.tx.try_send(Out::Close(Close::Gone));
            }
            !same
        });
        if inner.sessions.len() >= MAX_SESSIONS {
            return Err(Close::Busy);
        }
        let hello = json!({ "t": "hello", "v": VERSION, "min": MIN_VERSION, "name": inner.name });
        let intro = [
            Some(hello),
            state_json(&*self.host),
            Some(queue_json(&self.host.queue())),
            Some(library_json(&self.host.library())),
        ];
        for message in intro.into_iter().flatten() {
            let _ = tx.try_send(Out::Json(message));
        }
        inner.next_session += 1;
        let id = inner.next_session;
        inner.last_seen.insert(key.to_owned(), now_s());
        inner.sessions.insert(id, Session { key: key.to_owned(), tx });
        Ok(id)
    }

    fn unregister(&self, id: u64) {
        let mut inner = self.lock();
        if let Some(s) = inner.sessions.remove(&id) {
            inner.last_seen.insert(s.key, now_s());
        }
    }

    async fn session(&self, mut ws: Ws, mut transport: TransportState, key: String, ping: Option<u64>) {
        let (tx, mut rx) = mpsc::channel(SESSION_QUEUE);
        let id = match self.register(&key, tx.clone()) {
            Ok(id) => id,
            Err(why) => return close(&mut ws, why).await,
        };
        if let Some(n) = ping {
            let _ = tx.try_send(Out::Json(json!({ "t": "pong", "n": n, "at": now_ms() })));
        }
        drop(tx);
        self.changed();
        let mut buf = vec![0u8; NOISE_MAX];
        let mut deadline = Instant::now() + SILENCE;
        let why = loop {
            let step = {
                let incoming = pin!(timeout_at(deadline, ws.next()));
                let outgoing = pin!(rx.recv());
                match select(incoming, outgoing).await {
                    Either::Left((next, _)) => Either::Left(next),
                    Either::Right((out, _)) => Either::Right(out),
                }
            };
            let reply = match step {
                Either::Right(Some(Out::Json(message))) => Some(message),
                Either::Right(Some(Out::Close(why))) => break why,
                // Dropped from the sessions while its queue was full.
                Either::Right(None) => break Close::Off,
                Either::Left(Err(_)) => break Close::Timeout,
                Either::Left(Ok(next)) => match frame(next) {
                    Err(why) => break why,
                    Ok(None) => None,
                    Ok(Some(bytes)) => {
                        // A replayed, reordered or forged message fails here (Noise nonces and tags).
                        let Ok(n) = transport.read_message(&bytes, &mut buf) else { break Close::Invalid };
                        deadline = Instant::now() + SILENCE;
                        self.handle(&buf[..n])
                    }
                },
            };
            if let Some(message) = reply
                && let Err(why) = send_json(&mut ws, &mut transport, &mut buf, &message).await
            {
                break why;
            }
        };
        self.unregister(id);
        self.changed();
        if why != Close::Gone && why != Close::Timeout {
            eprintln!("remote: session closed ({})", why.frame().map_or("", |f| f.1));
        }
        close(&mut ws, why).await;
    }

    /// The reply to one controller message, if it has one.
    fn handle(&self, plain: &[u8]) -> Option<Value> {
        let Ok(incoming) = serde_json::from_slice::<Incoming>(plain) else { return Some(error("invalid")) };
        match incoming {
            Incoming::Ping { n } => Some(json!({ "t": "pong", "n": n, "at": now_ms() })),
            Incoming::Cmd(command) if !command.valid() => Some(error("invalid")),
            Incoming::Cmd(command) if command.refused_during_ad() && self.host.state().is_some_and(|s| s.ad) => {
                Some(error("ad"))
            }
            Incoming::Cmd(command) => (!self.host.run(&command)).then(|| error("unavailable")),
        }
    }
}

type Ws = WebSocketStream<TcpStream>;

async fn accept(shared: Arc<Shared>, listener: std::net::TcpListener) {
    let listener = match TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => return eprintln!("remote: {e}"),
    };
    loop {
        match listener.accept().await {
            Ok((tcp, peer)) => {
                async_runtime::spawn(connection(shared.clone(), tcp, peer));
            }
            Err(e) => {
                eprintln!("remote: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn connection(shared: Arc<Shared>, tcp: TcpStream, peer: SocketAddr) {
    // Dropped before any handshake: a source outside the local network, or too many tries from one address.
    if !allowed_source(peer.ip()) || !shared.admit(peer.ip()) {
        return;
    }
    let _ = tcp.set_nodelay(true);
    let mut pair = None;
    // The error type is tungstenite's (`Callback`).
    #[allow(clippy::result_large_err)]
    let callback = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        match request.uri().path() {
            "/v1" => pair = Some(false),
            "/v1/pair" => pair = Some(true),
            _ => return Err(http::Response::builder().status(404).body(None).expect("static response")),
        }
        Ok(response)
    };
    let config = WebSocketConfig::default().max_message_size(Some(NOISE_MAX)).max_frame_size(Some(NOISE_MAX));
    let accepted =
        timeout(HANDSHAKE_TIMEOUT, tokio_tungstenite::accept_hdr_async_with_config(tcp, callback, Some(config))).await;
    let (Ok(Ok(mut ws)), Some(pair)) = (accepted, pair) else { return };
    let result = if pair { shared.pair(&mut ws).await } else { shared.knock(&mut ws).await };
    match result {
        Ok((transport, key, ping)) => shared.session(ws, transport, key, ping).await,
        Err(why) => {
            // Logged without data: no address, key or name.
            let kind = if pair { "pairing" } else { "session" };
            eprintln!("remote: {kind} handshake refused ({})", why.frame().map_or("gone", |f| f.1));
            close(&mut ws, why).await;
        }
    }
}

async fn forward_states(shared: Arc<Shared>, mut rx: broadcast::Receiver<Event>) {
    while let Ok(_) | Err(RecvError::Lagged(_)) = rx.recv().await {
        if let Some(state) = state_json(&*shared.host) {
            shared.broadcast(state);
        }
    }
}

async fn forward_queues(shared: Arc<Shared>, mut rx: broadcast::Receiver<Queue>) {
    loop {
        match rx.recv().await {
            Ok(queue) => shared.broadcast(queue_json(&queue)),
            Err(RecvError::Lagged(_)) => shared.broadcast(queue_json(&shared.host.queue())),
            Err(RecvError::Closed) => break,
        }
    }
}

async fn forward_library(shared: Arc<Shared>, mut rx: broadcast::Receiver<Vec<Playlist>>) {
    loop {
        match rx.recv().await {
            Ok(library) => shared.broadcast(library_json(&library)),
            Err(RecvError::Lagged(_)) => shared.broadcast(library_json(&shared.host.library())),
            Err(RecvError::Closed) => break,
        }
    }
}

/// The bytes of a binary frame; `None` for a control frame, which is skipped.
fn frame(next: Option<Result<Message, tungstenite::Error>>) -> Result<Option<Vec<u8>>, Close> {
    match next {
        Some(Ok(Message::Binary(b))) if b.len() > MAX_FRAME => Err(Close::TooBig),
        Some(Ok(Message::Binary(b))) => Ok(Some(b.to_vec())),
        Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => Ok(None),
        Some(Ok(Message::Text(_))) => Err(Close::Invalid),
        Some(Ok(Message::Close(_)) | Err(_)) | None => Err(Close::Gone),
    }
}

async fn read_frame(ws: &mut Ws, until: Instant) -> Result<Vec<u8>, Close> {
    loop {
        let next = timeout_at(until, ws.next()).await.map_err(|_| Close::Timeout)?;
        if let Some(bytes) = frame(next)? {
            return Ok(bytes);
        }
    }
}

async fn send_raw(ws: &mut Ws, bytes: &[u8]) -> Result<(), Close> {
    ws.send(Message::binary(bytes.to_vec())).await.map_err(|_| Close::Gone)
}

async fn send_json(ws: &mut Ws, transport: &mut TransportState, buf: &mut [u8], message: &Value) -> Result<(), Close> {
    let text = message.to_string();
    if text.len() > MAX_OUT {
        eprintln!("remote: a message over 64 KB was not sent");
        return Ok(());
    }
    let n = transport.write_message(text.as_bytes(), buf).map_err(|_| Close::Invalid)?;
    send_raw(ws, &buf[..n]).await
}

/// Sends the close frame (when there is one to send) and waits briefly for the phone's.
async fn close(ws: &mut Ws, why: Close) {
    let Some((code, reason)) = why.frame() else { return };
    let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
    let _ = timeout(Duration::from_secs(2), async {
        let _ = ws.close(Some(frame)).await;
        while let Some(Ok(_)) = ws.next().await {}
    })
    .await;
}

fn state_json(host: &dyn Host) -> Option<Value> {
    let s = host.state()?;
    let t = &s.track;
    Some(json!({
        "t": "state",
        "track": {
            "id": t.id, "title": t.title, "artist": t.artist, "album": t.album, "artwork": t.artwork,
            "duration": t.duration,
        },
        "position": s.position_now(),
        "paused": s.paused,
        "ad": s.ad,
        "volume": s.volume,
        "muted": s.muted,
        "colors": host.colors(),
        "at": now_ms(),
    }))
}

/// The queue message, cut from the end until it fits in one Noise message (200 long titles may not).
fn queue_json(queue: &Queue) -> Value {
    let mut items = queue.items.clone();
    let mut current = queue.current;
    loop {
        let message = json!({ "t": "queue", "items": items, "current": current });
        if items.is_empty() || message.to_string().len() <= MAX_OUT {
            return message;
        }
        items.truncate(items.len() * 3 / 4);
        current = current.filter(|&i| i < items.len());
    }
}

/// The library message: at most 100 playlists of 200 characters, so it always fits.
fn library_json(library: &[Playlist]) -> Value {
    json!({ "t": "library", "items": library })
}

fn error(code: &str) -> Value {
    json!({ "t": "error", "code": code })
}

/// Private, link-local, loopback and CGNAT (mesh VPNs) sources; IPv4-mapped IPv6 counts as IPv4.
fn allowed_source(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (a == 100 && b & 0xc0 == 64)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => allowed_source(v4.into()),
            None => {
                let first = v6.segments()[0];
                v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
            }
        },
    }
}

/// The address of the interface the default route uses, found without sending anything (connecting a UDP
/// socket only picks a route).
fn lan_addresses() -> Vec<IpAddr> {
    let ip = UdpSocket::bind(("0.0.0.0", 0))
        .and_then(|s| {
            s.connect(("192.0.2.1", 9))?;
            s.local_addr()
        })
        .map(|a| a.ip());
    ip.into_iter().filter(|ip| allowed_source(*ip) && !ip.is_loopback()).collect()
}

fn pairing_uri(addrs: &[IpAddr], port: u16, key: &[u8; 32], secret: &[u8; 32]) -> String {
    let addrs: Vec<String> = addrs.iter().map(IpAddr::to_string).collect();
    format!("feathertune://pair?v={QR_VERSION}&k={}&s={}&a={}&p={port}", hex(key), hex(secret), addrs.join(","))
}

/// The 6 digits both screens show: the first 4 bytes of the handshake hash, big-endian, modulo 1 000 000.
fn pairing_code(hash: &[u8]) -> String {
    let n = u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]);
    format!("{:06}", n % 1_000_000)
}

pub fn valid_name(name: &str) -> Option<String> {
    let name = name.trim();
    let count = name.chars().count();
    ((1..=MAX_NAME).contains(&count) && !name.chars().any(char::is_control)).then(|| name.to_owned())
}

fn params(pattern: &str) -> snow::params::NoiseParams {
    pattern.parse().expect("valid Noise pattern")
}

fn random32() -> Result<[u8; 32], String> {
    use snow::resolvers::{CryptoResolver, DefaultResolver};
    let mut out = [0u8; 32];
    DefaultResolver
        .resolve_rng()
        .ok_or("no random source")?
        .try_fill_bytes(&mut out)
        .map_err(|e| format!("random source: {e:?}"))?;
    Ok(out)
}

fn load_keys() -> Result<Keys, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("credential store: {e}"))?;
    match entry.get_password() {
        Ok(text) => match Keys::from_text(&text) {
            Some(keys) => return Ok(keys),
            None => eprintln!("remote: the stored key is invalid; making a new one"),
        },
        Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(format!("credential store: {e}")),
    }
    let keys = Keys::generate();
    store_keys(&keys)?;
    Ok(keys)
}

fn store_keys(keys: &Keys) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .and_then(|e| e.set_password(&keys.to_text()))
        .map_err(|e| format!("credential store: {e}"))
}

/// The paired list; entries that do not check out are dropped.
fn load_devices(path: &Path) -> Vec<Device> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    match serde_json::from_str::<Vec<Device>>(&text) {
        Ok(devices) => devices
            .into_iter()
            .filter(|d| {
                unhex32(&d.key).is_some() && d.key == d.key.to_ascii_lowercase() && valid_name(&d.name).is_some()
            })
            .take(MAX_DEVICES)
            .collect(),
        Err(e) => {
            eprintln!("remote: {DEVICES_FILE} is invalid: {e}");
            Vec::new()
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn now_s() -> u64 {
    now_ms() / 1000
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::async_runtime::block_on;

    use super::*;
    use crate::player::{QueueItem, Track};

    struct TestHost {
        state: Mutex<Option<State>>,
        events: broadcast::Sender<Event>,
        queues: broadcast::Sender<Queue>,
        libraries: broadcast::Sender<Vec<Playlist>>,
        ran: Mutex<Vec<String>>,
    }

    impl TestHost {
        fn set_state(&self, state: State) {
            *self.state.lock().unwrap() = Some(state.clone());
            let _ = self.events.send(Event::Progress(state));
        }

        fn ran(&self) -> Vec<String> {
            self.ran.lock().unwrap().clone()
        }
    }

    impl Host for TestHost {
        fn state(&self) -> Option<State> {
            self.state.lock().unwrap().clone()
        }

        fn queue(&self) -> Queue {
            Queue {
                items: vec![QueueItem { id: "aaaaaaaaaaa".into(), title: "Q".into(), artist: "R".into() }],
                current: Some(0),
            }
        }

        fn subscribe(&self) -> broadcast::Receiver<Event> {
            self.events.subscribe()
        }

        fn subscribe_queue(&self) -> broadcast::Receiver<Queue> {
            self.queues.subscribe()
        }

        fn library(&self) -> Vec<Playlist> {
            vec![Playlist { list: "LM".into(), title: "Liked".into() }]
        }

        fn subscribe_library(&self) -> broadcast::Receiver<Vec<Playlist>> {
            self.libraries.subscribe()
        }

        fn colors(&self) -> Vec<String> {
            vec!["#112233".into()]
        }

        fn run(&self, command: &Command) -> bool {
            self.ran.lock().unwrap().push(format!("{command:?}"));
            true
        }
    }

    fn state(ad: bool) -> State {
        State {
            track: Track {
                id: Some("dQw4w9WgXcQ".into()),
                title: "T".into(),
                artist: "A".into(),
                album: String::new(),
                artwork: None,
                duration: 200.0,
            },
            position: 3.0,
            paused: false,
            volume: 40,
            muted: false,
            ad,
            at: std::time::Instant::now(),
        }
    }

    fn server() -> (Arc<Shared>, Arc<TestHost>, SocketAddr) {
        static N: AtomicUsize = AtomicUsize::new(0);
        let host = Arc::new(TestHost {
            state: Mutex::new(None),
            events: broadcast::channel(8).0,
            queues: broadcast::channel(8).0,
            libraries: broadcast::channel(4).0,
            ran: Mutex::new(Vec::new()),
        });
        let dir = std::env::temp_dir().join(format!(
            "feathertune-remote-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let shared = Arc::new(Shared::new(host.clone(), dir.join(DEVICES_FILE), false));
        let port = shared.listen(&["127.0.0.1:0".parse().unwrap()]).unwrap();
        (shared, host, SocketAddr::from(([127, 0, 0, 1], port)))
    }

    fn pc_key(shared: &Shared) -> [u8; 32] {
        shared.lock().keys.as_ref().unwrap().public
    }

    /// A phone's side of a session.
    struct Phone {
        ws: Ws,
        transport: TransportState,
        code: String,
        knocked: bool,
    }

    impl Phone {
        fn encrypt(&mut self, plain: &[u8]) -> Vec<u8> {
            let mut buf = vec![0u8; NOISE_MAX];
            let n = self.transport.write_message(plain, &mut buf).unwrap();
            buf[..n].to_vec()
        }

        async fn send_raw(&mut self, frame: Vec<u8>) {
            self.ws.send(Message::binary(frame)).await.unwrap();
        }

        async fn send(&mut self, plain: &str) {
            let frame = self.encrypt(plain.as_bytes());
            self.send_raw(frame).await;
        }

        /// The next message, or the close code.
        async fn recv(&mut self) -> Result<Value, Option<u16>> {
            let frame = read(&mut self.ws).await?;
            let mut buf = vec![0u8; NOISE_MAX];
            let n = self.transport.read_message(&frame, &mut buf).unwrap();
            Ok(serde_json::from_slice(&buf[..n]).unwrap())
        }

        /// Skips messages until the PC closes; its close code.
        async fn closed(&mut self) -> Option<u16> {
            loop {
                if let Err(code) = self.recv().await {
                    return code;
                }
            }
        }

        /// Reads `hello`, `state` (when one is set), `queue` and, after a knock, the pong to its first ping.
        async fn intro(&mut self) -> Vec<Value> {
            let mut out = Vec::new();
            let last = if self.knocked { "pong" } else { "library" };
            loop {
                let m = self.recv().await.unwrap();
                let done = m["t"] == last;
                out.push(m);
                if done {
                    return out;
                }
            }
        }
    }

    async fn read(ws: &mut Ws) -> Result<Vec<u8>, Option<u16>> {
        loop {
            match timeout(Duration::from_secs(5), ws.next()).await.expect("the PC answers in time") {
                Some(Ok(Message::Binary(b))) => return Ok(b.to_vec()),
                Some(Ok(Message::Close(f))) => return Err(f.map(|f| u16::from(f.code))),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return Err(None),
            }
        }
    }

    async fn open(addr: SocketAddr, path: &str) -> Ws {
        let tcp = TcpStream::connect(addr).await.unwrap();
        tokio_tungstenite::client_async(format!("ws://{addr}{path}"), tcp).await.unwrap().0
    }

    /// `IKpsk2` as the phone runs it, up to its proof; the PC then asks the user.
    async fn pair(
        addr: SocketAddr,
        pc: &[u8; 32],
        secret: &[u8; 32],
        phone: &Keys,
        name: &str,
    ) -> Result<Phone, Option<u16>> {
        let mut ws = open(addr, "/v1/pair").await;
        let mut hs = snow::Builder::new(params(PAIR_PATTERN))
            .local_private_key(&phone.private)
            .and_then(|b| b.remote_public_key(pc))
            .and_then(|b| b.psk(2, secret))
            .and_then(|b| b.prologue(PAIR_PROLOGUE))
            .and_then(|b| b.build_initiator())
            .unwrap();
        let mut buf = vec![0u8; NOISE_MAX];
        let n = hs.write_message(json!({ "name": name }).to_string().as_bytes(), &mut buf).unwrap();
        ws.send(Message::binary(buf[..n].to_vec())).await.unwrap();
        let reply = read(&mut ws).await?;
        hs.read_message(&reply, &mut buf).unwrap();
        let code = pairing_code(hs.get_handshake_hash());
        let mut phone = Phone { ws, transport: hs.into_transport_mode().unwrap(), code, knocked: false };
        phone.send(r#"{"t":"pair"}"#).await;
        Ok(phone)
    }

    /// `KK` as the phone runs it, then its first ping.
    async fn knock(addr: SocketAddr, pc: &[u8; 32], phone: &Keys) -> Result<Phone, Option<u16>> {
        let (mut ws, mut hs) = knock_start(addr, pc, phone).await;
        let mut buf = vec![0u8; NOISE_MAX];
        let reply = read(&mut ws).await?;
        hs.read_message(&reply, &mut buf).unwrap();
        let mut phone = Phone { ws, transport: hs.into_transport_mode().unwrap(), code: String::new(), knocked: true };
        phone.send(r#"{"t":"ping","n":0}"#).await;
        Ok(phone)
    }

    /// Opens `/v1` and sends `KK`'s first message.
    async fn knock_start(addr: SocketAddr, pc: &[u8; 32], phone: &Keys) -> (Ws, snow::HandshakeState) {
        let mut ws = open(addr, "/v1").await;
        let mut hs = snow::Builder::new(params(KK_PATTERN))
            .local_private_key(&phone.private)
            .and_then(|b| b.remote_public_key(pc))
            .and_then(|b| b.prologue(KK_PROLOGUE))
            .and_then(|b| b.build_initiator())
            .unwrap();
        let mut buf = vec![0u8; NOISE_MAX];
        let n = hs.write_message(&[], &mut buf).unwrap();
        ws.send(Message::binary(buf[..n].to_vec())).await.unwrap();
        (ws, hs)
    }

    async fn asked(shared: &Shared) -> Pending {
        for _ in 0..250 {
            if let Some(p) = shared.pending() {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no pending request");
    }

    /// A phone paired and allowed (its pairing session dropped).
    async fn paired(shared: &Shared, addr: SocketAddr) -> Keys {
        let keys = Keys::generate();
        let (_, pc, secret) = shared.new_pairing().unwrap();
        let mut phone = pair(addr, &pc, &secret, &keys, "Phone").await.unwrap();
        asked(shared).await;
        assert!(shared.answer(true));
        assert_eq!(phone.recv().await.unwrap()["t"], "hello");
        keys
    }

    #[test]
    fn pairing_with_the_matching_code_stores_the_phone_and_starts_a_session() {
        block_on(async {
            let (shared, _host, addr) = server();
            assert!(shared.pending().is_none());
            let (_, pc, secret) = shared.new_pairing().unwrap();
            let keys = Keys::generate();
            let mut phone = pair(addr, &pc, &secret, &keys, " Pixel 9 ").await.unwrap();
            assert_eq!(asked(&shared).await, Pending { name: "Pixel 9".into(), code: phone.code.clone() });
            assert_eq!(phone.code.len(), 6);
            assert!(shared.answer(true));
            let intro = phone.intro().await;
            assert_eq!(intro[0], json!({ "t": "hello", "v": 2, "min": 1, "name": "feathertune" }));
            assert_eq!(intro[intro.len() - 2]["items"][0]["id"], "aaaaaaaaaaa");
            assert_eq!(intro.last().unwrap()["items"][0]["list"], "LM");
            let devices = shared.devices();
            assert_eq!(devices.len(), 1);
            assert_eq!((devices[0].name.as_str(), devices[0].key.clone()), ("Pixel 9", hex(&keys.public)));
            assert!(devices[0].connected);
            assert!(shared.pending().is_none());
            // The file holds names, keys and dates only, and is read back.
            let stored = load_devices(&shared.devices_file);
            assert_eq!((stored.len(), stored[0].key.clone()), (1, hex(&keys.public)));
        });
    }

    #[test]
    fn a_denied_pairing_stores_nothing() {
        block_on(async {
            let (shared, _host, addr) = server();
            let (_, pc, secret) = shared.new_pairing().unwrap();
            let mut phone = pair(addr, &pc, &secret, &Keys::generate(), "Pixel").await.unwrap();
            asked(&shared).await;
            assert!(shared.answer(false));
            assert_eq!(phone.closed().await, Some(4000));
            assert!(shared.devices().is_empty());
            assert!(!shared.answer(true));
        });
    }

    #[test]
    fn an_expired_or_used_secret_is_refused() {
        block_on(async {
            let (shared, _host, addr) = server();
            // The 2 minutes are over.
            let (_, pc, secret) = shared.new_pairing().unwrap();
            shared.lock().pairing.as_mut().unwrap().until = Instant::now() - Duration::from_secs(1);
            assert_eq!(pair(addr, &pc, &secret, &Keys::generate(), "Late").await.err(), Some(Some(4001)));
            // A second phone with the secret already used, while the first waits for the answer and after it.
            let (_, pc, secret) = shared.new_pairing().unwrap();
            let mut first = pair(addr, &pc, &secret, &Keys::generate(), "First").await.unwrap();
            asked(&shared).await;
            assert_eq!(pair(addr, &pc, &secret, &Keys::generate(), "Copy").await.err(), Some(Some(4001)));
            assert!(shared.answer(true));
            assert_eq!(first.recv().await.unwrap()["t"], "hello");
            assert_eq!(pair(addr, &pc, &secret, &Keys::generate(), "Copy").await.err(), Some(Some(4001)));
            assert_eq!(shared.devices().len(), 1);
        });
    }

    #[test]
    fn junk_leaves_the_secret_but_a_first_message_made_with_the_pc_key_spends_it() {
        block_on(async {
            let (shared, _host, addr) = server();
            let (_, pc, secret) = shared.new_pairing().unwrap();
            // Anyone on the network, without the QR: refused, and the window stays open.
            let mut ws = open(addr, "/v1/pair").await;
            ws.send(Message::binary(vec![7u8; 96])).await.unwrap();
            assert_eq!(read(&mut ws).await.err(), Some(Some(4006)));
            // The PC's key but a wrong secret: message 1 reads (the secret only enters with the reply), so it is spent.
            let mut ws = open(addr, "/v1/pair").await;
            let mut hs = snow::Builder::new(params(PAIR_PATTERN))
                .local_private_key(&Keys::generate().private)
                .and_then(|b| b.remote_public_key(&pc))
                .and_then(|b| b.psk(2, &[0u8; 32]))
                .and_then(|b| b.prologue(PAIR_PROLOGUE))
                .and_then(|b| b.build_initiator())
                .unwrap();
            let mut buf = vec![0u8; NOISE_MAX];
            let n = hs.write_message(json!({ "name": "Guess" }).to_string().as_bytes(), &mut buf).unwrap();
            ws.send(Message::binary(buf[..n].to_vec())).await.unwrap();
            read(&mut ws).await.unwrap();
            drop(ws);
            assert_eq!(pair(addr, &pc, &secret, &Keys::generate(), "Late").await.err(), Some(Some(4001)));
            assert!(shared.devices().is_empty());
        });
    }

    #[test]
    fn junk_before_the_phone_does_not_stop_its_pairing() {
        block_on(async {
            let (shared, _host, addr) = server();
            let (_, pc, secret) = shared.new_pairing().unwrap();
            let mut ws = open(addr, "/v1/pair").await;
            ws.send(Message::binary(vec![7u8; 96])).await.unwrap();
            assert_eq!(read(&mut ws).await.err(), Some(Some(4006)));
            let mut phone = pair(addr, &pc, &secret, &Keys::generate(), "Phone").await.unwrap();
            asked(&shared).await;
            assert!(shared.answer(true));
            assert_eq!(phone.recv().await.unwrap()["t"], "hello");
        });
    }

    #[test]
    fn a_paired_phone_gets_the_state_and_its_commands_reach_the_player() {
        block_on(async {
            let (shared, host, addr) = server();
            let keys = paired(&shared, addr).await;
            host.set_state(state(false));
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            let intro = phone.intro().await;
            assert_eq!(
                intro.iter().map(|m| m["t"].as_str().unwrap()).collect::<Vec<_>>(),
                ["hello", "state", "queue", "library", "pong"]
            );
            let s = &intro[1];
            assert_eq!(
                (s["track"]["title"].as_str(), s["volume"].as_u64(), s["ad"].as_bool()),
                (Some("T"), Some(40), Some(false))
            );
            assert_eq!(s["colors"], json!(["#112233"]));
            phone.send(r#"{"t":"cmd","c":"next"}"#).await;
            phone.send(r#"{"t":"cmd","c":"seek","s":12.5}"#).await;
            phone.send(r#"{"t":"cmd","c":"play_item","id":"aaaaaaaaaaa","i":0}"#).await;
            phone.send(r#"{"t":"ping","n":7}"#).await;
            let pong = phone.recv().await.unwrap();
            assert_eq!((pong["t"].as_str(), pong["n"].as_u64()), (Some("pong"), Some(7)));
            assert_eq!(host.ran(), ["Next", "Seek { s: 12.5 }", r#"PlayItem { id: "aaaaaaaaaaa", i: 0 }"#]);
            assert_eq!(intro[3], json!({ "t": "library", "items": [{ "list": "LM", "title": "Liked" }] }));
            // A player event reaches the session.
            host.set_state(State { volume: 55, ..state(false) });
            assert_eq!(phone.recv().await.unwrap()["volume"], 55);
        });
    }

    #[test]
    fn queue_actions_and_playlists_are_checked_then_reach_the_player() {
        block_on(async {
            let (shared, host, addr) = server();
            let keys = paired(&shared, addr).await;
            host.set_state(state(false));
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            phone.intro().await;
            for bad in [
                r#""c":"queue_move","id":"aaaaaaaaaaa","i":2,"to":2"#,
                r#""c":"queue_move","id":"aaaaaaaaaaa","i":2,"to":200"#,
                r#""c":"queue_next","id":"short","i":0"#,
                r#""c":"queue_remove","id":"aaaaaaaaaaa","i":200"#,
                r#""c":"open_playlist","list":"../x""#,
                r#""c":"open_playlist","list":"LM","extra":1"#,
            ] {
                phone.send(&format!(r#"{{"t":"cmd",{bad}}}"#)).await;
                assert_eq!(phone.recv().await.unwrap(), json!({ "t": "error", "code": "invalid" }), "{bad}");
            }
            phone.send(r#"{"t":"cmd","c":"queue_move","id":"aaaaaaaaaaa","i":3,"to":0}"#).await;
            phone.send(r#"{"t":"cmd","c":"queue_next","id":"aaaaaaaaaaa","i":1}"#).await;
            phone.send(r#"{"t":"cmd","c":"queue_remove","id":"aaaaaaaaaaa","i":1}"#).await;
            phone.send(r#"{"t":"cmd","c":"open_playlist","list":"PLx_1-2"}"#).await;
            phone.send(r#"{"t":"ping","n":2}"#).await;
            assert_eq!(phone.recv().await.unwrap()["t"], "pong");
            assert_eq!(
                host.ran(),
                [
                    r#"QueueMove { id: "aaaaaaaaaaa", i: 3, to: 0 }"#,
                    r#"QueueNext { id: "aaaaaaaaaaa", i: 1 }"#,
                    r#"QueueRemove { id: "aaaaaaaaaaa", i: 1 }"#,
                    r#"OpenPlaylist { list: "PLx_1-2" }"#,
                ]
            );
            // A library change reaches the session.
            let _ = host.libraries.send(vec![]);
            assert_eq!(phone.recv().await.unwrap(), json!({ "t": "library", "items": [] }));

            host.set_state(state(true));
            assert_eq!(phone.recv().await.unwrap()["ad"], true);
            for c in [
                r#""c":"queue_next","id":"aaaaaaaaaaa","i":1"#,
                r#""c":"queue_move","id":"aaaaaaaaaaa","i":3,"to":0"#,
                r#""c":"open_playlist","list":"LM""#,
            ] {
                phone.send(&format!(r#"{{"t":"cmd",{c}}}"#)).await;
                assert_eq!(phone.recv().await.unwrap(), json!({ "t": "error", "code": "ad" }), "{c}");
            }
        });
    }

    #[test]
    fn commands_but_play_and_pause_are_refused_during_an_ad() {
        block_on(async {
            let (shared, host, addr) = server();
            let keys = paired(&shared, addr).await;
            host.set_state(state(true));
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            phone.intro().await;
            for c in [r#""c":"next""#, r#""c":"previous""#, r#""c":"seek","s":5"#, r#""c":"volume","v":10"#] {
                phone.send(&format!(r#"{{"t":"cmd",{c}}}"#)).await;
                assert_eq!(phone.recv().await.unwrap(), json!({ "t": "error", "code": "ad" }), "{c}");
            }
            phone.send(r#"{"t":"cmd","c":"pause"}"#).await;
            phone.send(r#"{"t":"ping","n":1}"#).await;
            assert_eq!(phone.recv().await.unwrap()["t"], "pong");
            assert_eq!(host.ran(), ["Pause"]);
        });
    }

    #[test]
    fn an_unknown_key_gets_nothing() {
        block_on(async {
            let (shared, _host, addr) = server();
            paired(&shared, addr).await;
            assert_eq!(knock(addr, &pc_key(&shared), &Keys::generate()).await.err(), Some(Some(4002)));
        });
    }

    #[test]
    fn a_removed_phone_is_closed_at_once_and_forget_all_drops_every_key() {
        block_on(async {
            let (shared, _host, addr) = server();
            let keys = paired(&shared, addr).await;
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            phone.intro().await;
            assert!(shared.remove(&hex(&keys.public)));
            assert_eq!(phone.closed().await, Some(4003));
            assert!(shared.devices().is_empty());
            assert_eq!(knock(addr, &pc_key(&shared), &keys).await.err(), Some(Some(4002)));

            let keys = paired(&shared, addr).await;
            let old = pc_key(&shared);
            shared.forget_all().unwrap();
            assert_ne!(pc_key(&shared), old);
            assert!(shared.devices().is_empty());
            assert_eq!(knock(addr, &pc_key(&shared), &keys).await.err(), Some(Some(4002)));
        });
    }

    #[test]
    fn a_replayed_message_closes_the_session_and_runs_once() {
        block_on(async {
            let (shared, host, addr) = server();
            let keys = paired(&shared, addr).await;
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            phone.intro().await;
            let frame = phone.encrypt(br#"{"t":"cmd","c":"next"}"#);
            phone.send_raw(frame.clone()).await;
            phone.send_raw(frame).await;
            assert_eq!(phone.closed().await, Some(4006));
            assert_eq!(host.ran(), ["Next"]);
        });
    }

    #[test]
    fn a_replayed_handshake_gets_nothing() {
        block_on(async {
            let (shared, _host, addr) = server();
            let keys = paired(&shared, addr).await;
            let pc = pc_key(&shared);
            let mut hs = snow::Builder::new(params(KK_PATTERN))
                .local_private_key(&keys.private)
                .and_then(|b| b.remote_public_key(&pc))
                .and_then(|b| b.prologue(KK_PROLOGUE))
                .and_then(|b| b.build_initiator())
                .unwrap();
            let mut buf = vec![0u8; NOISE_MAX];
            let n = hs.write_message(&[], &mut buf).unwrap();
            let first = buf[..n].to_vec();
            // A copy of the phone's first message: the PC cannot tell and answers the handshake, but without the
            // phone's keys the copy cannot send the ping, and nothing else comes before the close.
            let mut copy = open(addr, "/v1").await;
            copy.send(Message::binary(first)).await.unwrap();
            assert!(read(&mut copy).await.is_ok());
            copy.send(Message::binary(vec![0u8; 40])).await.unwrap();
            assert_eq!(read(&mut copy).await, Err(Some(4006)));
        });
    }

    #[test]
    fn malformed_commands_are_refused_and_an_oversized_one_closes() {
        block_on(async {
            let (shared, host, addr) = server();
            let keys = paired(&shared, addr).await;
            let mut phone = knock(addr, &pc_key(&shared), &keys).await.unwrap();
            phone.intro().await;
            for bad in [
                "not json",
                r#"{"t":"bogus"}"#,
                r#"{"t":"cmd","c":"jump"}"#,
                r#"{"t":"cmd","c":"next","x":1}"#,
                r#"{"t":"cmd","c":"seek","s":"x"}"#,
                r#"{"t":"cmd","c":"seek","s":-1}"#,
                r#"{"t":"cmd","c":"seek","s":1e9}"#,
                r#"{"t":"cmd","c":"volume","v":101}"#,
                r#"{"t":"cmd","c":"play_item","id":"bad","i":0}"#,
                r#"{"t":"cmd","c":"play_item","id":"aaaaaaaaaaa","i":200}"#,
                r#"{"t":"cmd","c":"play_item","id":"aaaaaaaaaaa"}"#,
                r#"{"t":"ping"}"#,
                r#"{"t":"ping","n":1,"c":"next"}"#,
            ] {
                phone.send(bad).await;
                assert_eq!(phone.recv().await.unwrap(), json!({ "t": "error", "code": "invalid" }), "{bad}");
            }
            assert!(host.ran().is_empty());
            phone.send(&json!({ "t": "ping", "n": 1, "pad": "x".repeat(MAX_IN) }).to_string()).await;
            assert_eq!(phone.closed().await, Some(4007));
        });
    }

    #[test]
    fn only_local_network_sources_are_accepted() {
        for ok in [
            "192.168.1.20",
            "10.0.0.5",
            "172.16.3.4",
            "172.31.255.1",
            "169.254.1.1",
            "127.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "::1",
            "fd12::1",
            "fe80::1",
            "::ffff:192.168.0.9",
        ] {
            assert!(allowed_source(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "172.32.0.1",
            "100.128.0.1",
            "100.63.255.255",
            "11.0.0.1",
            "2001:db8::1",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
            "0.0.0.0",
            "::",
        ] {
            assert!(!allowed_source(bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn handshakes_are_rate_limited_per_address() {
        block_on(async {
            let (shared, _host, _addr) = server();
            let ip: IpAddr = "192.168.1.9".parse().unwrap();
            for _ in 0..HANDSHAKES_PER_MINUTE {
                assert!(shared.admit(ip));
            }
            assert!(!shared.admit(ip));
            assert!(shared.admit("192.168.1.10".parse().unwrap()));
        });
    }

    #[test]
    fn code_uri_keys_and_names() {
        assert_eq!(pairing_code(&[0, 0, 0, 5, 9]), "000005");
        assert_eq!(pairing_code(&[0xff; 4]), "967295");
        let (key, secret) = ([0xab; 32], [0x01; 32]);
        assert_eq!(
            pairing_uri(&["192.168.1.20".parse().unwrap()], 47652, &key, &secret),
            format!("feathertune://pair?v=1&k={}&s={}&a=192.168.1.20&p=47652", "ab".repeat(32), "01".repeat(32))
        );
        let keys = Keys::generate();
        let back = Keys::from_text(&keys.to_text()).unwrap();
        assert_eq!((back.private, back.public), (keys.private, keys.public));
        assert!(Keys::from_text(&"é".repeat(64)).is_none());
        assert!(unhex32(&format!("+f{}", "0".repeat(62))).is_none());
        assert_eq!(valid_name(&"x".repeat(33)), None);
        assert_eq!(valid_name("a\u{7}b"), None);
        assert_eq!(valid_name("  "), None);
    }
}
