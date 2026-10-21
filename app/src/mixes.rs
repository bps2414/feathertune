//! Mixes: transitions per playlist and per pair of tracks, edited in the Mix tab of the settings
//! window and kept in `mixes/<list>.json` in the profile:
//!
//! ```json
//! { "enabled": true,
//!   "defaults": { "auto": true, "duration_ms": 6000, "curve": "equal_power", "effects": {} },
//!   "pairs": [{ "from": "<id>", "to": "<id>", "out_ms": 201000, "in_ms": 3000, "duration_ms": 8000,
//!               "curve": "s_curve", "effects": { "echo": true } }] }
//! ```
//!
//! For the current track and the next one the page gets one transition (`plan`): the pair's, else
//! the playlist's when its mix is on (auto: from both tracks' analysis), else none (the global
//! crossfade). Where the player plays from (playlist, next track, queue) comes from `player_state`
//! (`observe`) and is only used as data: to pick the transition and to list the playlist in the Mix
//! tab. The page's only other request here is `mix_open`, which opens the Mix tab on a pair.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, Webview};
use tauri_plugin_opener::OpenerExt;

use crate::audio_facts::{Facts, Store as FactsStore};
use crate::player::{Context, QueueItem, valid_id, valid_list};
use crate::settings::Live;

pub const MAX_PAIRS: usize = 2000;
/// 32 bars at 128 BPM is about this long.
pub const MAX_MIX_MS: u32 = 60_000;
const MAX_FILE: usize = 1024 * 1024;
const MAX_TRACK_MS: u32 = 86_400_000;
/// The auto mode: 16 beats of the outgoing track, within these bounds, else this length.
const AUTO_BEATS: f64 = 16.0;
const AUTO_MIN_MS: f64 = 4000.0;
const AUTO_MAX_MS: f64 = 12_000.0;
const AUTO_DEFAULT_MS: u32 = 6000;
const OPEN_EVERY: Duration = Duration::from_secs(1);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Effects {
    /// An echo on the outgoing tail.
    pub echo: bool,
    /// A high-pass rising on the outgoing track.
    pub filter: bool,
    /// The lows of the incoming track held down until halfway, then swapped (needs the overlap).
    pub bass_swap: bool,
    /// The incoming track at the outgoing one's tempo (needs the overlap).
    pub tempo: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    /// Length and points from the tracks' analysis (see `auto`); the other fields are the fallback.
    #[serde(default)]
    pub auto: bool,
    pub duration_ms: u32,
    pub curve: String,
    #[serde(default)]
    pub effects: Effects,
}

impl Default for Transition {
    fn default() -> Self {
        Transition {
            auto: true,
            duration_ms: AUTO_DEFAULT_MS,
            curve: "equal_power".into(),
            effects: Effects::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub from: String,
    pub to: String,
    /// Where the outgoing track starts to fade; none: its end minus the fade.
    #[serde(default)]
    pub out_ms: Option<u32>,
    /// Where the incoming track starts.
    #[serde(default)]
    pub in_ms: u32,
    pub duration_ms: u32,
    pub curve: String,
    #[serde(default)]
    pub effects: Effects,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub defaults: Transition,
    #[serde(default)]
    pub pairs: Vec<Pair>,
}

impl Mix {
    pub fn pair(&self, from: &str, to: &str) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.from == from && p.to == to)
    }
}

fn check_transition(duration_ms: u32, curve: &str) -> Result<(), String> {
    if duration_ms > MAX_MIX_MS {
        return Err(format!("a transition is at most {} s", MAX_MIX_MS / 1000));
    }
    if !crate::audio::CURVES.contains(&curve) {
        return Err("unknown curve".into());
    }
    Ok(())
}

pub fn check_pair(p: &Pair) -> Result<(), String> {
    if !valid_id(&p.from) || !valid_id(&p.to) || p.from == p.to {
        return Err("invalid track ids".into());
    }
    if p.out_ms.is_some_and(|o| o > MAX_TRACK_MS) || p.in_ms > MAX_TRACK_MS {
        return Err("a point is out of range".into());
    }
    check_transition(p.duration_ms, &p.curve)
}

/// Every field in range, at most `MAX_PAIRS` pairs and one per (from, to).
pub fn check(mix: &Mix) -> Result<(), String> {
    check_transition(mix.defaults.duration_ms, &mix.defaults.curve)?;
    if mix.pairs.len() > MAX_PAIRS {
        return Err(format!("at most {MAX_PAIRS} transitions per playlist"));
    }
    for (i, p) in mix.pairs.iter().enumerate() {
        check_pair(p)?;
        if mix.pairs[..i].iter().any(|q| q.from == p.from && q.to == p.to) {
            return Err("a transition is there twice".into());
        }
    }
    Ok(())
}

pub fn parse(json: &str) -> Result<Mix, String> {
    if json.len() > MAX_FILE {
        return Err("too large".into());
    }
    let mix: Mix = serde_json::from_str(json).map_err(|e| format!("not a mix: {e}"))?;
    check(&mix)?;
    Ok(mix)
}

/// The transition the page plays from `from` (A) into `to` (B), or `None` for the global crossfade.
pub fn plan(mix: Option<&Mix>, from: &str, to: &str, a: Option<&Facts>, b: Option<&Facts>) -> Option<Value> {
    let mix = mix.filter(|m| m.enabled)?;
    if let Some(p) = mix.pair(from, to) {
        return Some(json!({
            "mode": "pair", "from": from, "to": to, "out_ms": p.out_ms, "in_ms": p.in_ms,
            "duration_ms": p.duration_ms, "curve": p.curve, "effects": p.effects,
        }));
    }
    let d = &mix.defaults;
    let (out_ms, in_ms, duration_ms, curve) = if d.auto {
        let (out, duration) = auto(a);
        (out, b.map_or(0, |b| b.lead_in_ms.round() as u32), duration, "equal_power")
    } else {
        (None, 0, d.duration_ms, d.curve.as_str())
    };
    Some(json!({
        "mode": if d.auto { "auto" } else { "playlist" }, "from": from, "to": to, "out_ms": out_ms,
        "in_ms": in_ms, "duration_ms": duration_ms, "curve": curve, "effects": d.effects,
    }))
}

/// The auto mode: 16 beats of A (4 to 12 s), ending before A's final silence and starting on a beat.
/// Without A's tempo: 6 s from the end.
fn auto(a: Option<&Facts>) -> (Option<u32>, u32) {
    let Some(a) = a else { return (None, AUTO_DEFAULT_MS) };
    let end = a.tail_ms.unwrap_or(a.duration_s * 1000.0);
    let Some(bpm) = a.bpm else {
        return (Some((end - f64::from(AUTO_DEFAULT_MS)).max(0.0) as u32), AUTO_DEFAULT_MS);
    };
    let beat = 60_000.0 / bpm;
    let duration = (AUTO_BEATS * beat).clamp(AUTO_MIN_MS, AUTO_MAX_MS);
    let mut out = end - duration;
    if let Some(offset) = a.beat_offset_ms {
        out = offset + ((out - offset) / beat).floor() * beat;
    }
    (Some(out.max(0.0) as u32), duration.round() as u32)
}

#[derive(Default)]
struct Inner {
    list: Option<String>,
    next: Option<QueueItem>,
    queues: HashMap<String, Vec<QueueItem>>,
    open: Option<(String, String, String)>,
    last_open: Option<Instant>,
    seq: u64,
    command: Option<Value>,
}

pub struct Mixes {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl Mixes {
    pub fn new(profile: &Path) -> Mixes {
        Mixes { dir: profile.join("mixes"), inner: Mutex::default() }
    }

    fn path(&self, list: &str) -> PathBuf {
        self.dir.join(format!("{list}.json"))
    }

    pub fn load(&self, list: &str) -> Option<Mix> {
        if !valid_list(list) {
            return None;
        }
        let text = std::fs::read_to_string(self.path(list)).ok()?;
        parse(&text).map_err(|e| eprintln!("mixes/{list}.json: {e}")).ok()
    }

    fn save(&self, list: &str, mix: &Mix) -> Result<(), String> {
        check(mix)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(mix).expect("a mix serializes");
        std::fs::write(self.path(list), json).map_err(|e| format!("could not save the mix: {e}"))
    }

    /// The playlists whose mix is on, each with its edited pairs as "from>to" (for the page's buttons).
    pub fn enabled(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return out };
        for path in entries.flatten().map(|e| e.path()) {
            let Some(list) = path.file_stem().and_then(|s| s.to_str()).filter(|l| valid_list(l)) else { continue };
            if let Some(mix) = self.load(list).filter(|m| m.enabled) {
                let pairs: Vec<String> = mix.pairs.iter().map(|p| format!("{}>{}", p.from, p.to)).collect();
                out.insert(list.to_owned(), json!(pairs));
            }
        }
        out
    }

    /// The playlist and next track the player is on now.
    pub fn now(&self) -> (Option<String>, Option<QueueItem>) {
        let inner = self.inner.lock().unwrap();
        (inner.list.clone(), inner.next.clone())
    }

    /// A command for the page, once: `{ seq, issued_ms, <name>: data }` (see the audio module).
    pub fn command(&self, name: &str, data: Value) {
        let mut inner = self.inner.lock().unwrap();
        inner.seq += 1;
        let issued = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        inner.command = Some(json!({ "seq": inner.seq, "issued_ms": issued, name: data }));
    }

    pub fn pending_command(&self) -> Option<Value> {
        self.inner.lock().unwrap().command.clone()
    }
}

/// `player_state`'s context: stores it and gives the page a new transition when it changed.
pub fn observe(app: &AppHandle, context: Context) {
    let Some(mixes) = app.try_state::<Mixes>() else { return };
    let changed = {
        let mut inner = mixes.inner.lock().unwrap();
        if let (Some(list), Some(queue)) = (&context.list, context.queue) {
            inner.queues.insert(list.clone(), queue);
        }
        let changed = inner.list != context.list || inner.next != context.next;
        inner.list = context.list;
        inner.next = context.next;
        changed
    };
    if changed {
        crate::audio::push(app);
    }
}

/// The site asks to open the Mix tab on a pair (its button between two tracks). It only opens our
/// window: nothing is saved and the player is not touched. At most once a second.
#[tauri::command]
pub fn mix_open(webview: Webview, app: AppHandle, list: String, from: String, to: String) {
    if webview.label() != "site" || !valid_list(&list) || !valid_id(&from) || !valid_id(&to) {
        return;
    }
    let Some(mixes) = app.try_state::<Mixes>() else { return };
    {
        let mut inner = mixes.inner.lock().unwrap();
        if inner.last_open.is_some_and(|t| t.elapsed() < OPEN_EVERY) {
            return;
        }
        inner.last_open = Some(Instant::now());
        inner.open = Some((list, from, to));
    }
    crate::settings_window::open(&app);
}

/// The Mix tab. `op`: `get` ({list?}), `facts` ({id}), `save_pair` ({list, pair}), `delete_pair`
/// ({list, from, to}), `enable` ({list, on}), `defaults` ({list, defaults}), `preview` ({list, pair}),
/// `export` ({list}), `import` ({list, json}). Only the settings window may call it.
#[tauri::command]
pub fn settings_mix(webview: Webview, app: AppHandle, op: String, arg: Value) -> Result<Value, String> {
    if webview.label() != "settings" {
        return Err("not allowed".into());
    }
    let mixes = app.try_state::<Mixes>().ok_or("mixes are not running")?;
    let facts = app.state::<FactsStore>();
    let list_arg = || {
        arg.get("list").and_then(Value::as_str).filter(|l| valid_list(l)).map(str::to_owned).ok_or("invalid playlist")
    };
    let id_arg = |key: &str| arg.get(key).and_then(Value::as_str).filter(|i| valid_id(i)).ok_or("invalid track id");
    // The page gets the new transition and the buttons on the site follow the mixes that are on.
    let changed = |_: &str| crate::audio::push(&app);
    match op.as_str() {
        "get" => {
            let (open, current, queues) = {
                let mut inner = mixes.inner.lock().unwrap();
                (inner.open.take(), inner.list.clone(), inner.queues.clone())
            };
            // A pair the site asked to open wins over the playlist the tab shows.
            let list = open
                .as_ref()
                .map(|(l, _, _)| l.clone())
                .or_else(|| arg.get("list").and_then(Value::as_str).map(str::to_owned))
                .or_else(|| current.clone());
            let list = list.filter(|l| valid_list(l));
            let queue = list.as_ref().and_then(|l| queues.get(l)).cloned().unwrap_or_default();
            let summary: Vec<Value> = queue
                .iter()
                .map(|item| {
                    let f = facts.load(&item.id);
                    json!({
                        "id": item.id, "title": item.title, "artist": item.artist,
                        "measured": f.is_some(),
                        "duration_s": f.as_ref().map(|f| f.duration_s),
                        "bpm": f.as_ref().and_then(|f| f.bpm),
                        "camelot": f.as_ref().and_then(|f| f.camelot.clone()),
                    })
                })
                .collect();
            let mix = list.as_ref().and_then(|l| mixes.load(l)).unwrap_or_default();
            let lists: Vec<String> = queues.keys().cloned().chain(mixes.enabled().keys().cloned()).collect();
            Ok(json!({
                "open": open.map(|(list, from, to)| json!({ "list": list, "from": from, "to": to })),
                "current": current,
                "list": list,
                "lists": lists,
                "queue": summary,
                "mix": mix,
                "max_ms": MAX_MIX_MS,
            }))
        }
        "facts" => Ok(serde_json::to_value(facts.load(id_arg("id")?)).unwrap_or(Value::Null)),
        "save_pair" | "preview" => {
            let list = list_arg()?;
            let pair: Pair = serde_json::from_value(arg.get("pair").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("invalid transition: {e}"))?;
            check_pair(&pair)?;
            if op == "preview" {
                mixes.command("preview", serde_json::to_value(&pair).expect("a pair serializes"));
                crate::audio::push(&app);
                return Ok(Value::Null);
            }
            let mut mix = mixes.load(&list).unwrap_or_default();
            mix.pairs.retain(|p| !(p.from == pair.from && p.to == pair.to));
            mix.pairs.push(pair);
            mixes.save(&list, &mix)?;
            changed(&list);
            Ok(Value::Null)
        }
        "delete_pair" => {
            let list = list_arg()?;
            let (from, to) = (id_arg("from")?, id_arg("to")?);
            let mut mix = mixes.load(&list).unwrap_or_default();
            mix.pairs.retain(|p| !(p.from == from && p.to == to));
            mixes.save(&list, &mix)?;
            changed(&list);
            Ok(Value::Null)
        }
        "enable" => {
            let list = list_arg()?;
            let on = arg.get("on").and_then(Value::as_bool).ok_or("expected true or false")?;
            let mut mix = mixes.load(&list).unwrap_or_default();
            mix.enabled = on;
            mixes.save(&list, &mix)?;
            changed(&list);
            Ok(Value::Null)
        }
        "defaults" => {
            let list = list_arg()?;
            let defaults: Transition = serde_json::from_value(arg.get("defaults").cloned().unwrap_or(Value::Null))
                .map_err(|e| format!("invalid transition: {e}"))?;
            let mut mix = mixes.load(&list).unwrap_or_default();
            mix.defaults = defaults;
            mixes.save(&list, &mix)?;
            changed(&list);
            Ok(Value::Null)
        }
        "export" => {
            let list = list_arg()?;
            let mix = mixes.load(&list).ok_or("this playlist has no mix yet")?;
            let file = crate::settings_window::downloads()?.join(format!("feathertune-mix-{list}.json"));
            std::fs::write(&file, serde_json::to_string_pretty(&mix).expect("a mix serializes"))
                .map_err(|e| format!("could not write the file: {e}"))?;
            app.opener().reveal_item_in_dir(&file).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "import" => {
            let list = list_arg()?;
            let mix = parse(arg.get("json").and_then(Value::as_str).ok_or("paste the mix\u{2019}s JSON")?)?;
            mixes.save(&list, &mix)?;
            changed(&list);
            Ok(Value::Null)
        }
        _ => Err("unknown operation".into()),
    }
}

/// Facts for the page: the current and next track's (without the envelope).
pub fn page_facts(app: &AppHandle, id: &str) -> Option<Value> {
    let mut facts = serde_json::to_value(app.state::<FactsStore>().load(id)?).ok()?;
    facts.as_object_mut()?.remove("envelope");
    Some(facts)
}

/// What the audio page module gets about mixes: the transition from `current` into the next
/// track, the next track's facts and the playlists whose mix is on.
pub fn page_data(app: &AppHandle, current: Option<&str>) -> Value {
    let Some(mixes) = app.try_state::<Mixes>() else { return json!({}) };
    let (list, next) = mixes.now();
    let next_id = next.as_ref().map(|n| n.id.as_str());
    let store = app.state::<FactsStore>();
    let transition = match (current, next_id) {
        (Some(from), Some(to)) if app.state::<Live>().plugin("crossfade") => {
            let mix = list.as_deref().and_then(|l| mixes.load(l));
            plan(mix.as_ref(), from, to, store.load(from).as_ref(), store.load(to).as_ref())
        }
        _ => None,
    };
    json!({
        "list": list,
        "next": next_id,
        "transition": transition,
        "mixes": mixes.enabled(),
        "command": mixes.pending_command(),
        "next_facts": next_id.and_then(|id| page_facts(app, id)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbb";

    fn facts(bpm: Option<f64>) -> Facts {
        Facts {
            id: A.into(),
            duration_s: 200.0,
            bpm,
            beat_offset_ms: bpm.map(|_| 250.0),
            key: None,
            camelot: None,
            lead_in_ms: 1200.0,
            tail_ms: Some(198_000.0),
            envelope: String::new(),
        }
    }

    #[test]
    fn mix_files_are_checked() {
        let ok = format!(
            r#"{{"enabled":true,"pairs":[{{"from":"{A}","to":"{B}","out_ms":1000,"in_ms":0,"duration_ms":8000,"curve":"linear","effects":{{"echo":true}}}}]}}"#
        );
        let mix = parse(&ok).unwrap();
        assert!(mix.enabled && mix.defaults.auto && mix.pair(A, B).unwrap().effects.echo);
        for bad in [
            ok.replace("linear", "wobbly"),
            ok.replace("8000", "90000"),
            ok.replace(B, "../x"),
            ok.replace(B, A),
            ok.replace(r#""echo":true"#, r#""laser":true"#),
            ok.replace(r#"]}"#, &format!(r#",{{"from":"{A}","to":"{B}","duration_ms":1,"curve":"linear"}}]}}"#)),
            "1".into(),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pair_wins_over_the_playlist_and_nothing_without_a_mix() {
        let mut mix = Mix { enabled: true, ..Mix::default() };
        assert!(plan(None, A, B, None, None).is_none());
        assert!(plan(Some(&Mix::default()), A, B, None, None).is_none());
        assert_eq!(plan(Some(&mix), A, B, None, None).unwrap()["mode"], "auto");
        mix.pairs.push(Pair {
            from: A.into(),
            to: B.into(),
            out_ms: Some(5),
            in_ms: 7,
            duration_ms: 9000,
            curve: "linear".into(),
            effects: Effects::default(),
        });
        let p = plan(Some(&mix), A, B, None, None).unwrap();
        assert_eq!((p["mode"].as_str(), p["out_ms"].as_u64(), p["in_ms"].as_u64()), (Some("pair"), Some(5), Some(7)));
        assert_eq!(plan(Some(&mix), B, A, None, None).unwrap()["mode"], "auto");
    }

    #[test]
    fn auto_is_sixteen_beats_on_the_beat_before_the_tail() {
        // 120 BPM: 16 beats are 8 s; the tail is at 198 s, so out is 190 s, on the beat grid of 250 ms + k * 500.
        let (out, duration) = auto(Some(&facts(Some(120.0))));
        assert_eq!(duration, 8000);
        assert_eq!(out, Some(189_750));
        // 60 BPM would be 16 s: capped at 12.
        assert_eq!(auto(Some(&facts(Some(60.0)))).1, 12_000);
        assert_eq!(auto(Some(&facts(None))), (Some(192_000), 6000));
        assert_eq!(auto(None), (None, 6000));
        let p = plan(Some(&Mix { enabled: true, ..Mix::default() }), A, B, None, Some(&facts(None))).unwrap();
        assert_eq!(p["in_ms"], 1200);
    }
}
