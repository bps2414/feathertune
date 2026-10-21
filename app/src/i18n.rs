//! The app's language (`[ui] language`: "auto" is the language of Windows' regional format, "en" or
//! "pt-BR"; read once at startup, so a change applies after a restart).
//!
//! - The English text in the code is the key: `t("Quit")` gives the translation of the current
//!   language, or the English when it has none. `tf` fills `{}` in order.
//! - A language is one JSON file in `locales/`, English → translation, compiled in. The settings page
//!   gets the same table (`strings`) and translates its own text with it; the page scripts get their
//!   few texts from Rust as data.

use std::collections::HashMap;
use std::sync::OnceLock;

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::w;

/// The languages of the app, by tag, with their own name (shown in the settings).
pub const LANGUAGES: &[(&str, &str)] = &[("en", "English"), ("pt-BR", "Português (Brasil)")];
const PT_BR: &str = include_str!("../locales/pt-BR.json");

/// The language in use and its table (`None` for English).
static CURRENT: OnceLock<(&'static str, Option<HashMap<String, String>>)> = OnceLock::new();

/// Picks the language from the setting; before this (and in tests) everything is English.
pub fn init(setting: &str) {
    let tag = match setting {
        "en" => "en",
        "pt-BR" => "pt-BR",
        _ if windows_language().as_deref() == Some("pt") => "pt-BR",
        _ => "en",
    };
    let table = (tag == "pt-BR").then(|| match serde_json::from_str(PT_BR) {
        Ok(table) => table,
        Err(e) => {
            eprintln!("locales/pt-BR.json: {e}");
            HashMap::new()
        }
    });
    let _ = CURRENT.set((tag, table));
}

/// The language tag in use ("en" or "pt-BR").
pub fn language() -> &'static str {
    CURRENT.get().map_or("en", |(tag, _)| tag)
}

/// The translation table in use, for the settings page; `None` for English.
pub fn strings() -> Option<&'static HashMap<String, String>> {
    CURRENT.get().and_then(|(_, table)| table.as_ref())
}

/// `en` in the current language.
pub fn t(en: &'static str) -> &'static str {
    strings().and_then(|s| s.get(en)).map_or(en, String::as_str)
}

/// `en` in the current language, with each `{}` replaced by the next of `args`.
pub fn tf(en: &'static str, args: &[&str]) -> String {
    let mut out = t(en).to_owned();
    for arg in args {
        if let Some(i) = out.find("{}") {
            out.replace_range(i..i + 2, arg);
        }
    }
    out
}

/// The language of Windows' regional format (Settings > Time & language), as an ISO 639 code.
pub fn windows_language() -> Option<String> {
    let mut buf = [0u16; 85];
    let mut size = (buf.len() * 2) as u32;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Control Panel\International"),
            w!("LocaleName"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result.is_err() {
        return None;
    }
    let name = String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]);
    let lang = name.trim_end_matches('\0').split(['-', '_']).next()?.to_ascii_lowercase();
    ((2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_lowercase())).then_some(lang)
}
