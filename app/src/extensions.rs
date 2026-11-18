//! Extensions: plugins made by anyone, imported by the user from a folder, a `.zip` or a single `.js`/`.css`
//! file (the settings window's Extensions tab), kept in `extensions/<id>/` in the profile. Nothing is
//! downloaded and there is no catalog. They are the user's own code: the app does not review them and does not
//! limit what their page code does (docs/DESIGN.md, Extensions).
//!
//! A package is a folder with `plugin.json` (the manifest, `Manifest`) and any of `page.js`, `style.css`,
//! `plugin.wasm` (wasm.rs) and `icon.png`. Its switch is `[extensions] "<id>" = true` in `config.toml` (off
//! until the user turns it on after the import), its settings `[extension_settings] "<id>" = { ... }`, checked
//! against the manifest. The set the page runs is read once at startup: an import, a removal or a switch applies
//! after a restart; the settings apply at once.
//!
//! Each extension with page code is its own initialization script (`page_scripts`), so a syntax error in one
//! leaves the others and the built-in modules running. The page API it gets is `injections/extensions/page.js`.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::settings::Settings;

/// The API version this app implements; a manifest that asks for a newer one is refused.
pub const API: u32 = 1;
pub const DIR: &str = "extensions";
pub const MAX_EXTENSIONS: usize = 100;
const MAX_PAGE_BYTES: usize = 2 << 20;
pub const MAX_WASM_BYTES: usize = 16 << 20;
const MAX_PACKAGE_BYTES: usize = 32 << 20;
const MAX_ICON_BYTES: usize = 64 << 10;
const MAX_MANIFEST_BYTES: usize = 256 << 10;
const MAX_SETTINGS: usize = 32;
const MAX_ERRORS: usize = 20;
/// The files a package may have; anything else in it is ignored.
const FILES: &[&str] = &["plugin.json", "page.js", "style.css", "plugin.wasm", "icon.png"];
pub const PERMISSIONS: &[&str] = &["player.control", "storage", "notify"];

#[derive(Deserialize)]
struct Manifest {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    api: u32,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    settings: Vec<SettingDef>,
    /// Language tag → English text → translation, for the name, description and setting labels.
    #[serde(default)]
    i18n: BTreeMap<String, BTreeMap<String, String>>,
}

/// One setting the manifest declares; the Extensions tab draws a field for it.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SettingDef {
    pub key: String,
    /// bool, number, enum, string, color.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub hint: String,
    pub default: Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub permissions: Vec<String>,
    pub settings: Vec<SettingDef>,
    i18n: BTreeMap<String, BTreeMap<String, String>>,
    pub js: Option<String>,
    pub css: Option<String>,
    pub wasm: bool,
    /// A `data:` URL.
    pub icon: Option<String>,
}

impl Extension {
    /// One of the manifest's texts in the app's language.
    pub fn tr<'a>(&'a self, text: &'a str) -> &'a str {
        self.i18n.get(crate::i18n::language()).and_then(|m| m.get(text)).map_or(text, String::as_str)
    }

    /// The settings' values: the stored ones that are still valid, else the defaults.
    pub fn values(&self, settings: &Settings) -> Map<String, Value> {
        let stored = settings.extension_settings(&self.id);
        self.settings
            .iter()
            .map(|def| {
                let v = stored.get(&def.key).and_then(toml_to_json).filter(|v| def.check(v).is_ok());
                (def.key.clone(), v.unwrap_or_else(|| def.default.clone()))
            })
            .collect()
    }
}

impl SettingDef {
    /// Checks a value of this setting.
    pub fn check(&self, v: &Value) -> Result<(), String> {
        let ok = match self.kind.as_str() {
            "bool" => v.is_boolean(),
            "number" => v
                .as_f64()
                .is_some_and(|n| n.is_finite() && self.min.is_none_or(|m| n >= m) && self.max.is_none_or(|m| n <= m)),
            "enum" => v.as_str().is_some_and(|s| self.options.iter().any(|o| o == s)),
            "string" => v.as_str().is_some_and(|s| {
                let cap = self.max.map_or(200, |m| m.clamp(1.0, 2000.0) as usize);
                s.chars().count() <= cap && !s.chars().any(char::is_control)
            }),
            "color" => v.as_str().is_some_and(is_color),
            _ => false,
        };
        if ok { Ok(()) } else { Err(format!("invalid value for {}", self.key)) }
    }

    fn validate(&self) -> Result<(), String> {
        if !self.key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || !(1..=32).contains(&self.key.len())
        {
            return Err(format!("setting key {:?}: 1 to 32 of a-z, 0-9 and _", self.key));
        }
        if !["bool", "number", "enum", "string", "color"].contains(&self.kind.as_str()) {
            return Err(format!("setting {}: unknown type {:?}", self.key, self.kind));
        }
        if self.kind == "enum" && (self.options.is_empty() || self.options.len() > 50) {
            return Err(format!("setting {}: an enum needs 1 to 50 options", self.key));
        }
        if self.options.iter().any(|o| o.is_empty() || !short(o, 64)) {
            return Err(format!("setting {}: options are 1 to 64 characters", self.key));
        }
        if self.min.is_some_and(|m| !m.is_finite()) || self.max.is_some_and(|m| !m.is_finite()) {
            return Err(format!("setting {}: min or max is not finite", self.key));
        }
        if self.step.is_some_and(|s| !s.is_finite() || s <= 0.0) {
            return Err(format!("setting {}: step must be positive", self.key));
        }
        if self.kind == "string" && self.max.is_some_and(|m| !(1.0..=2000.0).contains(&m)) {
            return Err(format!("setting {}: string max must be 1 to 2000", self.key));
        }
        if let (Some(min), Some(max)) = (self.min, self.max)
            && min > max
        {
            return Err(format!("setting {}: min is over max", self.key));
        }
        if !short(&self.label, 100) || !short(&self.hint, 300) {
            return Err(format!("setting {}: label or hint too long", self.key));
        }
        self.check(&self.default).map_err(|_| format!("setting {}: the default is not a valid value", self.key))
    }
}

fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn short(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(char::is_control)
}

fn toml_to_json(v: &toml::Value) -> Option<Value> {
    match v {
        toml::Value::Boolean(b) => Some(json!(b)),
        toml::Value::Integer(i) => Some(json!(i)),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number),
        toml::Value::String(s) => Some(json!(s)),
        _ => None,
    }
}

/// A setting's value as it goes into `config.toml` (checked first).
pub fn json_to_toml(v: &Value) -> Option<toml_edit::Value> {
    match v {
        Value::Bool(b) => Some((*b).into()),
        Value::Number(n) => n.as_i64().map(Into::into).or_else(|| n.as_f64().map(Into::into)),
        Value::String(s) => Some(s.as_str().into()),
        _ => None,
    }
}

/// `autor.nome`: 3 to 64 of a-z, 0-9, `_` and `-`, in at least two parts separated by dots (built-in plugin ids
/// have none, so they never collide).
/// Valid extension id: `author.name`, lowercase ASCII letters/digits/underscores/hyphens and not Windows-reserved.
pub fn valid_id(id: &str) -> bool {
    (3..=64).contains(&id.len())
        && id.contains('.')
        && id.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
        && !crate::player::windows_reserved(id)
}

fn valid_host(host: &str) -> bool {
    let host = host.strip_prefix("*.").unwrap_or(host);
    if !(3..=253).contains(&host.len()) || !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }
    let labels_ok = host.split('.').all(|l| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    let Some(tld) = host.rsplit('.').next() else {
        return false;
    };
    labels_ok
        && tld.len() >= 2
        && !tld.bytes().all(|b| b.is_ascii_digit())
        && !matches!(tld, "localhost" | "local" | "internal")
}

fn host_covers(granted: &str, requested: &str) -> bool {
    match granted.strip_prefix("*.") {
        Some(base) => {
            let req = requested.strip_prefix("*.").unwrap_or(requested);
            req == base || req.strip_suffix(base).is_some_and(|p| p.ends_with('.'))
        }
        None => granted == requested,
    }
}

fn permission_covered(prev: &[String], next: &str) -> bool {
    if let Some(req_host) = next.strip_prefix("net:") {
        prev.iter().filter_map(|p| p.strip_prefix("net:")).any(|granted| host_covers(granted, req_host))
    } else {
        prev.iter().any(|p| p == next)
    }
}

fn escalated(prev: &Extension, next: &Extension) -> bool {
    (prev.js.is_none() && next.js.is_some())
        || (!prev.wasm && next.wasm)
        || next.permissions.iter().any(|p| !permission_covered(&prev.permissions, p))
}

fn max_file_bytes(name: &str) -> usize {
    match name {
        "plugin.json" => MAX_MANIFEST_BYTES,
        "page.js" | "style.css" => MAX_PAGE_BYTES,
        "plugin.wasm" => MAX_WASM_BYTES,
        "icon.png" => MAX_ICON_BYTES,
        _ => 0,
    }
}

/// Builds an extension from a package's files (names at its root), checking everything.
fn parse(files: &BTreeMap<String, Vec<u8>>) -> Result<Extension, String> {
    let manifest = files.get("plugin.json").ok_or_else(|| crate::i18n::t("no plugin.json"))?;
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err("plugin.json is too large".into());
    }
    let m: Manifest = serde_json::from_slice(manifest).map_err(|e| format!("plugin.json: {e}"))?;
    if !valid_id(&m.id) {
        return Err(format!("id {:?}: like author.name, a-z, 0-9, _ and - only", m.id));
    }
    if m.api == 0 || m.api > API {
        return Err(format!("needs API {} (this app has {API}); update the app", m.api));
    }
    if m.name.trim().is_empty() || !short(&m.name, 64) {
        return Err("name: 1 to 64 characters".into());
    }
    if !(1..=32).contains(&m.version.len())
        || !m.version.bytes().all(|b| b.is_ascii_alphanumeric() || b".+-".contains(&b))
    {
        return Err(format!("version {:?}: like 1.2.0", m.version));
    }
    if !short(&m.author, 64) || !short(&m.description, 500) {
        return Err("author or description too long".into());
    }
    for p in &m.permissions {
        let ok =
            PERMISSIONS.contains(&p.as_str()) || p == "player.read" || p.strip_prefix("net:").is_some_and(valid_host);
        if !ok {
            return Err(format!("unknown permission {p:?}"));
        }
    }
    if m.settings.len() > MAX_SETTINGS {
        return Err(format!("at most {MAX_SETTINGS} settings"));
    }
    for (i, def) in m.settings.iter().enumerate() {
        def.validate()?;
        if m.settings[..i].iter().any(|d| d.key == def.key) {
            return Err(format!("setting {} is declared twice", def.key));
        }
    }
    for (lang, texts) in &m.i18n {
        if !crate::i18n::LANGUAGES.iter().any(|(tag, _)| tag == lang) || texts.len() > 200 {
            return Err(format!("i18n: unknown language {lang:?} or too many texts"));
        }
        if texts.iter().any(|(k, v)| !short(k, 500) || !short(v, 500)) {
            return Err("i18n: a text is too long".into());
        }
    }
    let text = |name: &str| -> Result<Option<String>, String> {
        files.get(name).map(|b| String::from_utf8(b.clone()).map_err(|_| format!("{name} is not UTF-8"))).transpose()
    };
    let (js, css) = (text("page.js")?, text("style.css")?);
    if js.as_ref().map_or(0, String::len) + css.as_ref().map_or(0, String::len) > MAX_PAGE_BYTES {
        return Err("page.js and style.css together are over 2 MB".into());
    }
    let wasm = match files.get("plugin.wasm") {
        Some(w) if w.len() > MAX_WASM_BYTES => return Err("plugin.wasm is over 16 MB".into()),
        Some(w) if !w.starts_with(b"\0asm") => return Err("plugin.wasm is not WebAssembly".into()),
        Some(_) => true,
        None => false,
    };
    let icon = match files.get("icon.png") {
        Some(i) if i.len() > MAX_ICON_BYTES || !i.starts_with(b"\x89PNG\r\n\x1a\n") => {
            return Err("icon.png: a PNG of at most 64 KB".into());
        }
        Some(i) => Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(i))),
        None => None,
    };
    if js.is_none() && css.is_none() && !wasm {
        return Err("nothing to run: no page.js, style.css or plugin.wasm".into());
    }
    Ok(Extension {
        id: m.id,
        name: m.name.trim().into(),
        version: m.version,
        author: m.author,
        description: m.description,
        permissions: m.permissions,
        settings: m.settings,
        i18n: m.i18n,
        js,
        css,
        wasm,
        icon,
    })
}

/// What `scan` found: the valid extensions, and the folders that are not (folder name, why).
#[derive(Default)]
pub struct Found {
    pub list: Vec<Arc<Extension>>,
    pub broken: Vec<(String, String)>,
}

pub fn scan(profile: &Path) -> Found {
    let mut found = Found::default();
    let Ok(entries) = std::fs::read_dir(profile.join(DIR)) else { return found };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| std::fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink() && m.file_type().is_dir()))
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let mut files = BTreeMap::new();
        let mut read_err = None;
        for f in FILES {
            let path = dir.join(f);
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    read_err = Some(format!("{f}: {e}"));
                    break;
                }
            };
            if meta.file_type().is_symlink() || !meta.file_type().is_file() {
                read_err = Some(format!("{f} is not a regular file"));
                break;
            }
            if meta.len() > max_file_bytes(f) as u64 {
                read_err = Some(format!("{f} is too large"));
                break;
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    files.insert((*f).to_owned(), bytes);
                }
                Err(e) => {
                    read_err = Some(format!("{f}: {e}"));
                    break;
                }
            }
        }
        if let Some(err) = read_err {
            found.broken.push((name, err));
            continue;
        }
        match parse(&files) {
            Ok(e) if e.id != name => found.broken.push((name, format!("the folder must be named {}", e.id))),
            Ok(_) if found.list.len() >= MAX_EXTENSIONS => {
                found.broken.push((name, format!("at most {MAX_EXTENSIONS} extensions")))
            }
            Ok(e) => found.list.push(Arc::new(e)),
            Err(e) => found.broken.push((name, e)),
        }
    }
    found
}

/// The extensions of this run.
pub struct Extensions {
    profile: PathBuf,
    /// The ones on at startup: what the page and the WebAssembly runtime run.
    running: Vec<Arc<Extension>>,
    /// What is on disk now (an import or a removal changes it).
    found: Mutex<Found>,
    /// The last errors each one reported, newest last.
    errors: Mutex<BTreeMap<String, VecDeque<String>>>,
}

impl Extensions {
    pub fn load(profile: &Path, settings: &Settings) -> Extensions {
        let found = scan(profile);
        let running = found.list.iter().filter(|e| settings.extension(&e.id)).cloned().collect();
        for (name, why) in &found.broken {
            crate::diag::log("extensions", format!("{name}: {why}"));
        }
        Extensions { profile: profile.to_owned(), running, found: Mutex::new(found), errors: Mutex::default() }
    }

    pub fn running(&self) -> &[Arc<Extension>] {
        &self.running
    }

    /// One that runs this time.
    pub fn get(&self, id: &str) -> Option<&Arc<Extension>> {
        self.running.iter().find(|e| e.id == id)
    }

    /// One on disk now (running or not).
    pub fn found(&self, id: &str) -> Option<Arc<Extension>> {
        self.found.lock().unwrap().list.iter().find(|e| e.id == id).cloned()
    }

    pub fn dir(&self) -> PathBuf {
        self.profile.join(DIR)
    }

    pub fn error(&self, id: &str, message: &str) {
        let message: String = message.chars().filter(|c| !c.is_control()).take(500).collect();
        crate::diag::log("extensions", format!("{id}: {message}"));
        let mut errors = self.errors.lock().unwrap();
        let list = errors.entry(id.to_owned()).or_default();
        if list.len() == MAX_ERRORS {
            list.pop_front();
        }
        list.push_back(message);
    }

    /// The Extensions tab's list.
    pub fn view(&self, settings: &Settings) -> Value {
        let found = self.found.lock().unwrap();
        let errors = self.errors.lock().unwrap();
        let list: Vec<Value> = found
            .list
            .iter()
            .map(|e| {
                let settings_view: Vec<Value> = e
                    .settings
                    .iter()
                    .map(|d| {
                        let mut v = serde_json::to_value(d).unwrap_or_default();
                        v["label"] = json!(e.tr(if d.label.is_empty() { &d.key } else { &d.label }));
                        v["hint"] = json!(e.tr(&d.hint));
                        v
                    })
                    .collect();
                json!({
                    "id": e.id,
                    "name": e.tr(&e.name),
                    "version": e.version,
                    "author": e.author,
                    "description": e.tr(&e.description),
                    "permissions": e.permissions,
                    "settings": settings_view,
                    "values": e.values(settings),
                    "icon": e.icon,
                    "page": e.js.is_some(),
                    "css": e.css.is_some(),
                    "wasm": e.wasm,
                    "on": settings.extension(&e.id),
                    // Not running yet: imported, updated or switched on since the start.
                    "running": self.running.iter().any(|r| Arc::ptr_eq(r, e)) && settings.extension(&e.id),
                    "errors": errors.get(&e.id).map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_default(),
                })
            })
            .collect();
        let broken: Vec<Value> = found.broken.iter().map(|(name, why)| json!({ "name": name, "error": why })).collect();
        json!({ "list": list, "broken": broken, "api": API })
    }

    /// Imports a package: `files` maps paths (as the user's folder or zip has them, or one file's name) to their
    /// bytes. Returns what the confirmation shows. An extension with the same id is replaced (an update); its
    /// switch, settings and stored data stay.
    pub fn import(&self, files: BTreeMap<String, Vec<u8>>) -> Result<Value, String> {
        let files = package(files)?;
        let ext = parse(&files)?;
        let prev = self.found(&ext.id);
        if prev.is_none() && self.found.lock().unwrap().list.len() >= MAX_EXTENSIONS {
            return Err(format!("at most {MAX_EXTENSIONS} extensions"));
        }
        let is_escalated = prev.as_ref().is_some_and(|p| escalated(p, &ext));
        let previous = prev.map(|e| e.version.clone());
        let dir = self.dir();
        let tmp = dir.join(format!(".import-{}", ext.id));
        let old = dir.join(format!(".old-{}", ext.id));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::remove_dir_all(&old);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        for (name, bytes) in &files {
            std::fs::write(tmp.join(name), bytes).map_err(|e| {
                let _ = std::fs::remove_dir_all(&tmp);
                e.to_string()
            })?;
        }
        let target = dir.join(&ext.id);
        let had_target = match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.file_type().is_dir() => {
                std::fs::remove_dir(&target).or_else(|_| std::fs::remove_file(&target)).map_err(|e| {
                    let _ = std::fs::remove_dir_all(&tmp);
                    format!("could not replace the old version: {e}")
                })?;
                false
            }
            Ok(_) => {
                std::fs::rename(&target, &old).map_err(|e| {
                    let _ = std::fs::remove_dir_all(&tmp);
                    format!("could not replace the old version: {e}")
                })?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(e.to_string());
            }
        };
        if let Err(e) = std::fs::rename(&tmp, &target) {
            if had_target {
                let _ = std::fs::rename(&old, &target);
            }
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e.to_string());
        }
        if had_target {
            let _ = std::fs::remove_dir_all(&old);
        }
        *self.found.lock().unwrap() = scan(&self.profile);
        Ok(json!({
            "id": ext.id,
            "name": ext.tr(&ext.name),
            "version": ext.version,
            "previous": previous,
            "escalated": is_escalated,
            "permissions": ext.permissions,
            "page": ext.js.is_some(),
            "wasm": ext.wasm,
        }))
    }

    /// Deletes an extension's folder and its stored data.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        if !valid_id(id) {
            return Err("unknown extension".into());
        }
        let dir = self.dir().join(id);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.file_type().is_dir() => {
                std::fs::remove_dir(&dir).or_else(|_| std::fs::remove_file(&dir)).map_err(|e| e.to_string())?;
            }
            Ok(_) => std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let data = self.profile.join(DATA_DIR).join(format!("{id}.json"));
        if std::fs::symlink_metadata(&data).is_ok_and(|m| !m.file_type().is_symlink()) {
            let _ = std::fs::remove_file(data);
        }
        *self.found.lock().unwrap() = scan(&self.profile);
        Ok(())
    }
}

/// Where each extension's `storage` lives (`<id>.json`).
pub const DATA_DIR: &str = "extension-data";

/// Turns what the user picked into a package's root files: a folder or zip with `plugin.json` at its root or in
/// one top folder, or a single `.css` (a theme) or `.js` with a `==Feathertune==` header.
fn package(files: BTreeMap<String, Vec<u8>>) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut clean = BTreeMap::new();
    for (path, bytes) in files {
        let path = path.replace('\\', "/");
        let path = path.trim_start_matches("./");
        if path.is_empty()
            || path.starts_with('/')
            || path.contains(':')
            || path.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        {
            return Err(format!("invalid path in the package: {path:?}"));
        }
        clean.insert(path.to_owned(), bytes);
    }
    if clean.len() == 1 && !clean.contains_key("plugin.json") {
        let (name, bytes) = clean.into_iter().next().expect("one file");
        return single_file(&name, bytes);
    }
    let prefix = if clean.contains_key("plugin.json") {
        String::new()
    } else {
        let mut tops = clean.keys().filter_map(|k| k.strip_suffix("/plugin.json")).filter(|top| !top.contains('/'));
        match (tops.next(), tops.next()) {
            (Some(top), None) => format!("{top}/"),
            _ => return Err(crate::i18n::t("no plugin.json at the top of the package, or more than one").into()),
        }
    };
    let mut kept = BTreeMap::new();
    let mut total = 0usize;
    for (path, bytes) in clean {
        let Some(name) = path.strip_prefix(&prefix) else { continue };
        if FILES.contains(&name) {
            total = total.saturating_add(bytes.len());
            if total > MAX_PACKAGE_BYTES {
                return Err(crate::i18n::t("The package is over 32 MB.").into());
            }
            kept.insert(name.to_owned(), bytes);
        }
    }
    Ok(kept)
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() { "extension".into() } else { s }
}

/// A single file as a package: a `.css` is a theme; a `.js` takes its manifest from a header:
/// `// ==Feathertune==`, then `// @id a.b`, `@name`, `@version`, `@author`, `@description`, `@permission x` (one
/// per line), then `// ==/Feathertune==`.
fn single_file(name: &str, bytes: Vec<u8>) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if bytes.len() > MAX_PAGE_BYTES {
        return Err("page.js and style.css together are over 2 MB".into());
    }
    let file = name.rsplit('/').next().unwrap_or(name);
    let (stem, ext) = file.rsplit_once('.').ok_or_else(|| crate::i18n::t("pick a folder, a .zip, a .js or a .css"))?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{file} is not UTF-8"))?;
    let default_name: String = stem.chars().filter(|c| !c.is_control()).take(64).collect();
    let default_name = default_name.trim();
    let default_name = if default_name.is_empty() { "Extension" } else { default_name };
    let mut manifest =
        json!({ "id": format!("local.{}", slug(stem)), "name": default_name, "version": "1.0.0", "api": API });
    let page = match ext.to_ascii_lowercase().as_str() {
        "css" => "style.css",
        "js" => {
            let mut permissions = Vec::new();
            let mut inside = false;
            for line in text.lines().map(str::trim) {
                let Some(line) = line.strip_prefix("//").map(str::trim) else {
                    if inside {
                        break;
                    }
                    continue;
                };
                match line {
                    "==Feathertune==" => inside = true,
                    "==/Feathertune==" => break,
                    _ if inside => {
                        if let Some((key, value)) =
                            line.strip_prefix('@').and_then(|l| l.split_once(char::is_whitespace))
                        {
                            let value = value.trim();
                            match key {
                                "permission" => permissions.push(value.to_owned()),
                                "id" | "name" | "version" | "author" | "description" => manifest[key] = json!(value),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            manifest["permissions"] = json!(permissions);
            "page.js"
        }
        _ => return Err(crate::i18n::t("pick a folder, a .zip, a .js or a .css").into()),
    };
    Ok(BTreeMap::from([
        ("plugin.json".to_owned(), manifest.to_string().into_bytes()),
        (page.to_owned(), text.into_bytes()),
    ]))
}

/// One initialization script per running extension with page code. Each runs after the built-in modules'
/// script, gets its API object from `feathertune.ext` (injections/extensions/page.js) and runs inside its own
/// `try/catch`; a syntax error only stops that one (the runtime reports it as not started).
pub fn page_scripts(running: &[Arc<Extension>], settings: &Settings) -> Vec<String> {
    running
        .iter()
        .filter(|e| e.js.is_some() || e.css.is_some())
        .map(|e| {
            let meta = json!({
                "id": e.id,
                "version": e.version,
                "permissions": e.permissions,
                "settings": e.values(settings),
                "native": e.wasm,
            });
            let meta = serde_json::to_string(&meta.to_string()).expect("a string serializes");
            let id = serde_json::to_string(&e.id).expect("a string serializes");
            let mut out = format!(
                "(() => {{\n\
                 if (location.origin !== 'https://music.youtube.com' || window.top !== window) return;\n\
                 const host = window.__feathertune?.ext;\n\
                 if (!host) return;\n\
                 const ft = host.context(JSON.parse({meta}));\n\
                 try {{\n"
            );
            if let Some(css) = &e.css {
                out.push_str(&format!("ft.addStyle({});\n", serde_json::to_string(css).expect("a string serializes")));
            }
            if let Some(js) = &e.js {
                out.push_str(&format!("((ft) => {{\n{js}\n}})(ft);\n"));
            }
            out.push_str(&format!("}} catch (e) {{ host.failed({id}, e); }}\n}})();\n"));
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("feathertune-ext-test-{label}-{nanos}"))
    }

    #[test]
    fn host_and_id_validation() {
        assert!(valid_host("api.example.com"));
        assert!(valid_host("*.cdn.example.co.uk"));
        assert!(!valid_host("127.0.0.1"));
        assert!(!valid_host("192.168.1.1"));
        assert!(!valid_host("foo.123"));
        assert!(!valid_host("foo.a"));
        assert!(!valid_host("app.localhost"));
        assert!(!valid_host("*.local"));
        assert!(!valid_host("svc.internal"));
        assert!(!valid_host("-bad.com"));
        assert!(!valid_host("bad-.com"));

        assert!(valid_id("author.cool-ext"));
        assert!(!valid_id("con.test"));
        assert!(!valid_id("nul.plugin"));
    }

    #[test]
    fn string_max_is_bounded_and_single_file_truncates_name() {
        let bad: SettingDef = serde_json::from_value(json!({
            "key": "s",
            "type": "string",
            "default": "ok",
            "max": 50000
        }))
        .unwrap();
        assert!(bad.validate().is_err());

        let neg: SettingDef = serde_json::from_value(json!({
            "key": "s",
            "type": "string",
            "default": "",
            "max": -5
        }))
        .unwrap();
        assert!(neg.validate().is_err());

        let long_name = format!("{}.css", "a".repeat(80));
        let pkg = single_file(&long_name, b"body {}".to_vec()).unwrap();
        let ext = parse(&pkg).unwrap();
        assert_eq!(ext.name.len(), 64);
    }

    #[test]
    fn package_ignores_extra_files_before_size_limit() {
        let manifest = br#"{"id":"test.pkg","name":"Test","version":"1.0.0","api":1}"#.to_vec();
        let big_extra = vec![b'x'; MAX_PACKAGE_BYTES + 1024];
        let pkg = package(BTreeMap::from([
            ("my-ext/plugin.json".into(), manifest),
            ("my-ext/style.css".into(), b"body {}".to_vec()),
            ("my-ext/target/giant.bin".into(), big_extra),
        ]))
        .unwrap();
        assert_eq!(pkg.len(), 2);
        assert!(parse(&pkg).is_ok());
    }

    #[test]
    fn import_detects_capability_escalation_and_swaps_atomically() {
        let dir = unique_dir("escalate");
        let _ = std::fs::remove_dir_all(&dir);
        let exts = Extensions {
            profile: dir.clone(),
            running: Vec::new(),
            found: Mutex::new(Found::default()),
            errors: Mutex::new(BTreeMap::new()),
        };

        let v1 = BTreeMap::from([
            (
                "plugin.json".into(),
                br#"{"id":"test.swap","name":"Swap","version":"1.0.0","api":1,"permissions":["storage","net:api.example.com"]}"#.to_vec(),
            ),
            ("page.js".into(), b"console.log(1);".to_vec()),
        ]);
        let r1 = exts.import(v1).unwrap();
        assert_eq!(r1["escalated"], json!(false));

        let v2_same = BTreeMap::from([
            (
                "plugin.json".into(),
                br#"{"id":"test.swap","name":"Swap","version":"1.0.1","api":1,"permissions":["storage","net:api.example.com"]}"#.to_vec(),
            ),
            ("page.js".into(), b"console.log(2);".to_vec()),
        ]);
        let r2 = exts.import(v2_same).unwrap();
        assert_eq!(r2["escalated"], json!(false));

        let v3_wild = BTreeMap::from([
            (
                "plugin.json".into(),
                br#"{"id":"test.swap","name":"Swap","version":"1.0.2","api":1,"permissions":["storage","net:*.example.com"]}"#.to_vec(),
            ),
            ("page.js".into(), b"console.log(3);".to_vec()),
        ]);
        let r3 = exts.import(v3_wild).unwrap();
        assert_eq!(r3["escalated"], json!(true));

        let v4_narrow = BTreeMap::from([
            (
                "plugin.json".into(),
                br#"{"id":"test.swap","name":"Swap","version":"1.0.3","api":1,"permissions":["storage","net:sub.example.com"]}"#.to_vec(),
            ),
            ("page.js".into(), b"console.log(4);".to_vec()),
        ]);
        let r4 = exts.import(v4_narrow).unwrap();
        assert_eq!(r4["escalated"], json!(false));

        let v5_wasm = BTreeMap::from([
            (
                "plugin.json".into(),
                br#"{"id":"test.swap","name":"Swap","version":"1.0.4","api":1,"permissions":["storage","net:sub.example.com"]}"#.to_vec(),
            ),
            ("page.js".into(), b"console.log(5);".to_vec()),
            ("plugin.wasm".into(), b"\0asm\x01\0\0\0".to_vec()),
        ]);
        let r5 = exts.import(v5_wasm).unwrap();
        assert_eq!(r5["escalated"], json!(true));
        assert!(!dir.join(DIR).join(".old-test.swap").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
