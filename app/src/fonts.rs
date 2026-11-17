//! Fonts for the Lyrics tab (`[lyrics] font`): the files in the profile's `fonts\` folder, added by
//! importing one (TTF, OTF, WOFF or WOFF2, at most 10 MB) or downloading one from the `LIBRARY` of free
//! fonts (SIL Open Font License, the latin subset as WOFF2 from the Fontsource CDN, on jsDelivr). The
//! chosen font goes to the page as bytes (`injections::send("lyricsFont", …)`), where lyrics/page.js
//! loads it with the FontFace API: the page never fetches anything for it. An empty name is the site's
//! own font. Call `download` from a worker thread.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::settings::Live;

const MAX_FONT_BYTES: u64 = 10 * 1024 * 1024;
const MAX_FONTS: usize = 64;
const EXTENSIONS: &[&str] = &["woff2", "woff", "ttf", "otf"];

/// Free fonts to download: (Fontsource id, name, kind, weight fetched).
pub const LIBRARY: &[(&str, &str, &str, u16)] = &[
    ("dancing-script", "Dancing Script", "script", 700),
    ("great-vibes", "Great Vibes", "script", 400),
    ("pacifico", "Pacifico", "script", 400),
    ("satisfy", "Satisfy", "script", 400),
    ("sacramento", "Sacramento", "script", 400),
    ("kaushan-script", "Kaushan Script", "script", 400),
    ("lobster", "Lobster", "script", 400),
    ("caveat", "Caveat", "handwriting", 700),
    ("permanent-marker", "Permanent Marker", "handwriting", 400),
    ("shadows-into-light", "Shadows Into Light", "handwriting", 400),
    ("indie-flower", "Indie Flower", "handwriting", 400),
    ("amatic-sc", "Amatic SC", "handwriting", 700),
    ("bebas-neue", "Bebas Neue", "display", 400),
    ("anton", "Anton", "display", 400),
    ("righteous", "Righteous", "display", 400),
    ("press-start-2p", "Press Start 2P", "display", 400),
    ("montserrat", "Montserrat", "sans", 800),
    ("poppins", "Poppins", "sans", 800),
    ("nunito", "Nunito", "sans", 800),
    ("inter", "Inter", "sans", 800),
    ("playfair-display", "Playfair Display", "serif", 800),
    ("merriweather", "Merriweather", "serif", 900),
    ("lora", "Lora", "serif", 700),
];

fn dir(app: &AppHandle) -> PathBuf {
    app.state::<Live>().profile.join("fonts")
}

/// A font's name as a file stem: letters, digits, spaces, `-` and `_`, 1 to 64 characters.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.trim() == name
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
}

/// The installed fonts: (name, file), by name.
fn installed(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter_map(|p| {
            let ext = p.extension()?.to_str()?.to_ascii_lowercase();
            let name = p.file_stem()?.to_str()?.to_owned();
            (EXTENSIONS.contains(&ext.as_str()) && valid_name(&name)).then_some((name, p))
        })
        .collect();
    out.sort_by_key(|a| a.0.to_lowercase());
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// What the settings window shows: the installed fonts and the library (each with whether it is in).
pub fn view(app: &AppHandle) -> Value {
    let names: Vec<String> = installed(&dir(app)).into_iter().map(|(n, _)| n).collect();
    let library: Vec<Value> = LIBRARY
        .iter()
        .map(|(id, name, kind, _)| json!({ "id": id, "name": name, "kind": kind, "installed": names.iter().any(|n| n == name) }))
        .collect();
    json!({ "installed": names, "library": library })
}

/// The file's kind from its first bytes: WOFF2, WOFF, TrueType or OpenType.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes.get(..4)? {
        b"wOF2" => Some("woff2"),
        b"wOFF" => Some("woff"),
        [0, 1, 0, 0] | b"true" => Some("ttf"),
        b"OTTO" => Some("otf"),
        _ => None,
    }
}

fn save(app: &AppHandle, name: &str, bytes: &[u8]) -> Result<(), String> {
    if !valid_name(name) {
        return Err("invalid font name".into());
    }
    if bytes.len() as u64 > MAX_FONT_BYTES {
        return Err("the font is over 10 MB".into());
    }
    let ext = sniff(bytes).ok_or("not a font file (TTF, OTF, WOFF or WOFF2)")?;
    let dir = dir(app);
    let fonts = installed(&dir);
    if fonts.len() >= MAX_FONTS && !fonts.iter().any(|(n, _)| n == name) {
        return Err(format!("at most {MAX_FONTS} fonts"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (n, path) in fonts {
        if n == name {
            let _ = std::fs::remove_file(path);
        }
    }
    std::fs::write(dir.join(format!("{name}.{ext}")), bytes).map_err(|e| e.to_string())
}

/// Imports a font file the settings window read (`file_name` gives the name).
pub fn import(app: &AppHandle, file_name: &str, bytes: &[u8]) -> Result<String, String> {
    let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let name: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { ' ' })
        .take(64)
        .collect::<String>()
        .trim()
        .to_owned();
    save(app, &name, bytes)?;
    Ok(name)
}

/// Downloads a font of the library.
pub fn download(app: &AppHandle, id: &str) -> Result<String, String> {
    let (id, name, _, weight) = LIBRARY.iter().find(|f| f.0 == id).ok_or("unknown font")?;
    let url = format!("https://cdn.jsdelivr.net/fontsource/fonts/{id}@latest/latin-{weight}-normal.woff2");
    let mut response = agent().get(&url).call().map_err(|e| e.to_string())?;
    if response.status() != 200 {
        return Err(format!("HTTP {}", response.status()));
    }
    let bytes = response.body_mut().with_config().limit(MAX_FONT_BYTES).read_to_vec().map_err(|e| e.to_string())?;
    save(app, name, &bytes)?;
    Ok((*name).to_owned())
}

/// Removes an installed font.
pub fn remove(app: &AppHandle, name: &str) -> Result<(), String> {
    for (n, path) in installed(&dir(app)) {
        if n == name {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Gives the page the chosen font (`[lyrics] font`), or null for the site's own.
pub fn push(app: &AppHandle) {
    let name = app.state::<Live>().get().lyrics.font.clone();
    let font = (!name.is_empty())
        .then(|| installed(&dir(app)).into_iter().find(|(n, _)| *n == name))
        .flatten()
        .and_then(|(n, path)| Some((n, std::fs::read(path).ok()?)));
    let data = font.map_or(
        Value::Null,
        |(name, bytes)| json!({ "name": name, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }),
    );
    crate::injections::send(app, "lyricsFont", &data);
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .tls_config(
                    TlsConfig::builder()
                        .provider(TlsProvider::NativeTls)
                        .root_certs(RootCerts::PlatformVerifier)
                        .build(),
                )
                .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION")))
                .max_redirects(0)
                .timeout_global(Some(Duration::from_secs(30)))
                .build(),
        )
    })
}
