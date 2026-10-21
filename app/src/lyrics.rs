//! Synced lyrics (plugin `synced_lyrics`): once per track Rust asks the lyrics sources for the timed
//! lyrics and hands the lines to the `lyrics` injection as data, which shows them in the site's Lyrics
//! tab following the <video> time. When no source has any the page is told nothing, so the site's own
//! lyrics stay as they are.
//!
//! - Sources (`[lyrics] providers`, read at each track, so a change applies at the next one), asked all
//!   at once, each on its own short-lived thread. The first answer with lines is shown at once, and a
//!   better one replaces it when it comes: lines with word times beat lines timed as a whole, then the
//!   order of the list decides. Every request names the app in its User-Agent.
//!   - `betterlyrics`: the public Better Lyrics API (https://docs.betterlyrics.org), timed word by word
//!     (TTML). Without an API key it answers only for tracks it already has cached; a 401 is a miss.
//!   - `unison`: Unison (https://docs.betterlyrics.org/unison), Better Lyrics' open database of
//!     community-synced lyrics, mostly word by word (TTML, or LRC). By the video id, else by title,
//!     artist and duration. Entries hidden by the community are skipped.
//!   - `lrcmux`: the public lrcmux API (https://lrcmux.dev), asked for word times only (`level=word`,
//!     `strict`), and never from the service's own lyrics (`sources=!ytmusic`) or Genius. An answer
//!     with garbled text (U+FFFD) is a miss.
//!   - `lrclib`: LRCLIB (https://lrclib.net), open and community-made, mostly timed by line. `GET
//!     /api/get` (title, artist, album when known, duration); when it has no synced lyrics, `GET
//!     /api/search` with the title cleaned of "(Official Video)", "feat. ..." and the like, and the best
//!     entry whose artist and title match and whose length is within `MAX_DURATION_GAP` s.
//! - It waits for the duration, which arrives a moment after the title.
//! - Answers are cached in `lyrics-cache/` in the profile, one small JSON file per track, so a track
//!   already seen costs no request. Lines with word times are kept while their source is on; lines timed
//!   as a whole and "not found" for a week, and only for the same list of sources (a source may get
//!   word times later). Failures (network, server busy) are not retried before the track changes.
//! - Lines timed only as a whole get estimated word times (`estimate`, `[lyrics] estimate_words`), and
//!   every time is moved by `[lyrics] offset_ms`. Both are applied to the lines kept from the source,
//!   so a change in the settings window shows at once (`republish`).
//! - The page never fetches anything and never calls Rust for this: it only receives
//!   `{ id, lines: [[ms, text, words?], ...], source, estimated }` through `injections::send`, or
//!   `null` on a track change. `words` (`[[ms, text], ...]`) is there only when the line has word times.
//! - Plain (unsynced) lyrics are ignored: the site already shows those.
//! - The lines of the current track are also kept in Rust (`Lyrics`, managed state) for the floating
//!   lyrics (desktop_lyrics.rs). The lookup runs when either plugin is on; the page gets the lines only
//!   with `synced_lyrics` on.
//! - Translation (plugin `lyrics_translation`, `[lyrics] translate_to`: "auto" is the language of
//!   Windows' regional format, "off", or a code of `TRANSLATION_LANGUAGES`): once the lines are final,
//!   a translation line by line into that language: the source's own (translations in its TTML, by
//!   people) when it covers half the lines, else Unison's `POST /translate` (machine, made for lyrics;
//!   only the lines' text and the language are sent). Cached with the lines, per language. Lines already
//!   in that language get none. The page gets `translation: [text | null, ...]` (one per line), and
//!   its source in `credit`; a new language applies from the next track, "off" at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tokio::sync::broadcast::error::RecvError;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::player::{Player, Track};
use crate::settings::{Live, Settings};

const LRCLIB: &str = "https://lrclib.net/api";
const BETTER_LYRICS: &str = "https://api.betterlyrics.org/getLyrics";
const UNISON: &str = "https://unison.boidu.dev/lyrics";
const UNISON_TRANSLATE: &str = "https://unison.boidu.dev/translate";
const LRCMUX: &str = "https://api.lrcmux.dev/get";
/// Unison translates at most this many lines a request.
const TRANSLATE_CHUNK: usize = 200;
/// Translations kept per line from a TTML source.
const MAX_TRANSLATIONS: usize = 4;
/// The languages lyrics can be translated to (`[lyrics] translate_to`), by ISO 639-1 code, with their
/// own name (shown in the settings).
pub const TRANSLATION_LANGUAGES: &[(&str, &str)] = &[
    ("pt", "Português"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("ko", "한국어"),
];
/// The sources, by id (`[lyrics] providers`) and name (shown in the page's credit and the settings),
/// in the default order: word by word first.
pub const PROVIDERS: &[(&str, &str)] =
    &[("betterlyrics", "Better Lyrics"), ("unison", "Unison"), ("lrcmux", "lrcmux"), ("lrclib", "LRCLIB")];
const USER_AGENT: &str =
    concat!("feathertune/", env!("CARGO_PKG_VERSION"), " (https://github.com/bps2414/feathertune)");
const MAX_BODY: u64 = 512 * 1024;
const MAX_LINES: usize = 2000;
const MAX_LINE_CHARS: usize = 300;
const MAX_WORDS: usize = 100;
/// A search result whose length differs from the track's by more than this (seconds) is not it.
const MAX_DURATION_GAP: f64 = 10.0;
const MAX_RESULTS: usize = 20;
/// A cached "no synced lyrics" is asked again after this long.
const NOT_FOUND_TTL: u64 = 7 * 24 * 3600;
/// The cache holds at most this many files; over it, the oldest are removed down to `KEEP_FILES`.
const MAX_FILES: usize = 400;
const KEEP_FILES: usize = 300;

pub type Words = Vec<(u32, String)>;
type Lines = Vec<Line>;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Line {
    pub ms: u32,
    pub text: String,
    /// Each word with its own time and the spaces after it; empty when the line is timed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Words,
    /// Translations that came with the lyrics (TTML), as (language tag, text).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<(String, String)>,
    /// The translation shown with the line (set when shown, never cached with the lines).
    #[serde(skip)]
    pub translation: Option<String>,
}

/// The synced lines of the track `key` (`track_key`), as shown: offset, and with estimated word times
/// when `estimated`.
pub struct Synced {
    pub key: String,
    pub lines: Vec<Line>,
    /// The source's name.
    pub source: &'static str,
    pub estimated: bool,
    /// The translation's source, when the lines carry one (`Line::translation`).
    pub translated: Option<&'static str>,
}

/// The lines of the current track as the source gave them.
struct Found {
    key: String,
    id: Option<String>,
    lines: Lines,
    source: &'static str,
    translation: Option<Translated>,
}

/// A translation of the found lines, one entry per line.
struct Translated {
    lang: String,
    lines: Vec<Option<String>>,
    source: &'static str,
}

/// The synced lyrics of the current track, `None` while there are none (managed state).
#[derive(Default)]
pub struct Lyrics {
    shown: Mutex<Option<Arc<Synced>>>,
    found: Mutex<Option<Found>>,
    /// The `synced_lyrics` plugin is on, so the page gets the lines too.
    page: AtomicBool,
}

impl Lyrics {
    pub fn current(&self) -> Option<Arc<Synced>> {
        self.shown.lock().unwrap().clone()
    }

    fn set(&self, synced: Option<Synced>) {
        crate::titlebar::set_lyrics(synced.as_ref().is_some_and(|s| !s.lines.is_empty()));
        *self.shown.lock().unwrap() = synced.map(Arc::new);
        crate::desktop_lyrics::refresh();
        crate::share_lyrics::refresh();
        crate::miniplayer::refresh();
    }
}

impl Line {
    fn to_json(&self) -> Value {
        if self.words.is_empty() { json!([self.ms, self.text]) } else { json!([self.ms, self.text, self.words]) }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Unix seconds.
    at: u64,
    /// `None`: no source has synced lyrics for the track.
    lines: Option<Lines>,
    /// The source id of `lines` (entries written before there were several sources: LRCLIB).
    #[serde(default)]
    source: String,
    /// The sources asked, for a "not found".
    #[serde(default)]
    tried: Vec<String>,
    /// Translations of `lines` from Unison, one per language.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    translations: Vec<CachedTranslation>,
}

#[derive(Serialize, Deserialize)]
struct CachedTranslation {
    lang: String,
    /// One per line; `None`: the line needs none.
    lines: Vec<Option<String>>,
}

/// `page`: the `synced_lyrics` plugin is on, so the page gets the lines too.
pub fn start(app: &AppHandle, profile: &Path, page: bool) {
    let app = app.clone();
    app.state::<Lyrics>().page.store(page, Ordering::Relaxed);
    let dir = profile.join("lyrics-cache");
    let mut rx = app.state::<Player>().subscribe();
    std::thread::spawn(move || {
        let agent = agent();
        // The track being shown, and whether it was already asked for.
        let mut current = String::new();
        let mut asked = false;
        loop {
            let state = match rx.blocking_recv() {
                Ok(event) => event.state().clone(),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            let key = track_key(&state.track);
            if key != current {
                current = key;
                asked = false;
                *app.state::<Lyrics>().found.lock().unwrap() = None;
                publish(&app);
            }
            // The duration arrives after the title, in a later event.
            if asked || state.track.duration <= 0.0 {
                continue;
            }
            asked = true;
            let providers = enabled(&app.state::<Live>().get().lyrics.providers);
            lookup(&agent, &dir, &state.track, &providers, |lines, source| {
                // The track may have changed during the requests.
                if app.state::<Player>().current().is_some_and(|s| track_key(&s.track) == current) {
                    *app.state::<Lyrics>().found.lock().unwrap() = Some(Found {
                        key: current.clone(),
                        id: state.track.id.clone(),
                        lines,
                        source,
                        translation: None,
                    });
                    publish(&app);
                }
            });
            let target = translation_target(&app.state::<Live>().get());
            if let Some(lang) = target {
                translate(&app, &agent, &dir, &state.track, &current, &lang);
            }
        }
    });
}

/// The language to translate lyrics to, when they are translated (`[lyrics] translate_to`).
pub fn translation_target(s: &Settings) -> Option<String> {
    if !s.plugin("lyrics_translation") {
        return None;
    }
    match s.lyrics.translate_to.as_str() {
        "off" => None,
        "auto" => crate::i18n::windows_language(),
        code => TRANSLATION_LANGUAGES.iter().any(|(c, _)| *c == code).then(|| code.to_owned()),
    }
}

/// Finds a translation into `lang` of the current lines (track `key`) and shows it: the source's own
/// when it covers half the lines, else the cached one, else Unison's.
fn translate(app: &AppHandle, agent: &ureq::Agent, dir: &Path, track: &Track, key: &str, lang: &str) {
    let found = app.state::<Lyrics>().found.lock().unwrap().as_ref().filter(|f| f.key == key).map(|f| {
        let own = own_translation(&f.lines, lang);
        (f.lines.clone(), f.source, own)
    });
    let Some((lines, source, own)) = found else { return };
    let translated = match own {
        Some(own) => Translated { lang: lang.to_owned(), lines: own, source },
        None => {
            let file = cache_file(dir, track, track.duration.round() as u32);
            let Some(machine) = machine_translation(agent, &file, &lines, lang) else { return };
            Translated { lang: lang.to_owned(), lines: machine, source: "Unison" }
        }
    };
    let lyrics = app.state::<Lyrics>();
    if let Some(f) = lyrics.found.lock().unwrap().as_mut().filter(|f| f.key == key && f.lines.len() == lines.len()) {
        f.translation = Some(translated);
    } else {
        return;
    }
    publish(app);
}

fn same_language(tag: &str, lang: &str) -> bool {
    tag.split(['-', '_']).next().is_some_and(|t| t.eq_ignore_ascii_case(lang))
}

/// The translation into `lang` that came with the lines, when it covers at least half the sung ones.
fn own_translation(lines: &[Line], lang: &str) -> Option<Vec<Option<String>>> {
    let out: Vec<Option<String>> = lines
        .iter()
        .map(|l| l.translations.iter().find(|(tag, _)| same_language(tag, lang)).map(|(_, t)| t.clone()))
        .collect();
    let sung = lines.iter().filter(|l| !l.text.trim().is_empty()).count();
    let covered = out.iter().filter(|t| t.is_some()).count();
    (covered > 0 && covered * 2 >= sung).then_some(out)
}

/// Unison's translation of the lines, from the cache entry `file` when it has one for `lang`.
fn machine_translation(agent: &ureq::Agent, file: &Path, lines: &[Line], lang: &str) -> Option<Vec<Option<String>>> {
    let mut entry = read_entry(file);
    if let Some(t) =
        entry.as_ref().and_then(|e| e.translations.iter().find(|t| t.lang == lang && t.lines.len() == lines.len()))
    {
        return Some(t.lines.clone());
    }
    let out = match unison_translate(agent, lines, lang) {
        Ok(out) => out,
        Err(e) => {
            eprintln!("lyrics: translation: {e}");
            return None;
        }
    };
    // Kept only with the very lines it translates.
    if let Some(entry) = entry.as_mut().filter(|e| e.lines.as_ref().is_some_and(|l| l.len() == lines.len())) {
        entry.translations.retain(|t| t.lang != lang);
        entry.translations.push(CachedTranslation { lang: lang.to_owned(), lines: out.clone() });
        write_entry(file, entry);
    }
    Some(out)
}

/// `POST /translate` on Unison, `TRANSLATE_CHUNK` lines at a time: one entry per line, `None` where the
/// line needs no translation (blank, or already in `lang`). A 400 (nothing to translate, or the language
/// not found) is no translation.
fn unison_translate(agent: &ureq::Agent, lines: &[Line], lang: &str) -> Result<Vec<Option<String>>, String> {
    let mut out = Vec::with_capacity(lines.len());
    for chunk in lines.chunks(TRANSLATE_CHUNK) {
        let texts: Vec<&str> = chunk.iter().map(|l| l.text.as_str()).collect();
        let body = json!({ "lines": texts, "to": lang }).to_string();
        let answer: Value = match agent.post(UNISON_TRANSLATE).header("content-type", "application/json").send(body) {
            Ok(mut response) => {
                let bytes =
                    response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;
                serde_json::from_slice(&bytes).map_err(|e| format!("unreadable answer: {e}"))?
            }
            Err(ureq::Error::StatusCode(400)) => {
                out.extend(chunk.iter().map(|_| None));
                continue;
            }
            Err(e) => return Err(format!("request failed: {e}")),
        };
        let items = answer
            .get("lines")
            .and_then(Value::as_array)
            .filter(|a| a.len() == chunk.len())
            .ok_or("lines do not match")?;
        for (item, line) in items.iter().zip(chunk) {
            let wanted = item.get("needsTranslation").and_then(Value::as_bool) == Some(true);
            let text = item.get("translation").and_then(Value::as_str).filter(|_| wanted).map(clean_text);
            out.push(text.filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case(&line.text)));
        }
    }
    Ok(out)
}

/// A line of text from a source: no control characters, trimmed and capped.
fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(MAX_LINE_CHARS).collect()
}

/// The HTTP client of the sources: the platform's TLS and roots, our User-Agent, no redirects, 10 s.
fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(USER_AGENT)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(10)))
            .build(),
    )
}

/// Shows the lines of the current track again with the settings of now (word estimate, offset).
pub fn republish(app: &AppHandle) {
    if app.try_state::<Lyrics>().is_some() {
        publish(app);
    }
}

/// Applies the settings to the lines found and hands them to the page and the native views.
fn publish(app: &AppHandle) {
    let (estimate_words, offset, target) = {
        let s = app.state::<Live>();
        let s = s.get();
        (s.lyrics.estimate_words, s.lyrics.offset_ms, translation_target(&s))
    };
    let lyrics = app.state::<Lyrics>();
    let synced = lyrics.found.lock().unwrap().as_ref().map(|f| {
        let mut lines = f.lines.clone();
        let estimated = estimate_words && estimate(&mut lines);
        shift(&mut lines, offset);
        let translation = f
            .translation
            .as_ref()
            .filter(|t| target.as_deref() == Some(t.lang.as_str()) && t.lines.iter().any(Option::is_some));
        if let Some(t) = translation {
            for (line, text) in lines.iter_mut().zip(&t.lines) {
                line.translation = text.clone();
            }
        }
        let translated = translation.map(|t| t.source);
        (f.id.clone(), Synced { key: f.key.clone(), lines, source: f.source, estimated, translated })
    });
    if lyrics.page.load(Ordering::Relaxed) {
        let payload = synced.as_ref().map_or(Value::Null, |(id, s)| {
            let lines: Vec<Value> = s.lines.iter().map(Line::to_json).collect();
            let mut credit = crate::i18n::tf("Synced lyrics: {}", &[s.source]);
            if s.estimated {
                credit = crate::i18n::tf("{} · word timing estimated", &[&credit]);
            }
            let mut payload = json!({ "id": id, "lines": lines, "estimated": s.estimated });
            if let Some(source) = s.translated {
                let translation: Vec<Option<&str>> = s.lines.iter().map(|l| l.translation.as_deref()).collect();
                payload["translation"] = json!(translation);
                credit = crate::i18n::tf("{} · Translation: {}", &[&credit, source]);
            }
            payload["credit"] = json!(credit);
            payload
        });
        crate::injections::send(app, "lyrics", &payload);
    }
    lyrics.set(synced.map(|(_, s)| s));
}

/// The known sources of `ids`, in order, once each.
fn enabled(ids: &[String]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for id in ids {
        if let Some((known, _)) = PROVIDERS.iter().find(|(p, _)| p == id)
            && !out.contains(known)
        {
            out.push(known);
        }
    }
    out
}

fn name(id: &str) -> &'static str {
    PROVIDERS.iter().find(|(p, _)| *p == id).map_or("LRCLIB", |(_, n)| n)
}

pub fn track_key(t: &Track) -> String {
    t.id.clone().unwrap_or_else(|| format!("{}\n{}", t.title, t.artist))
}

/// Finds the synced lines of a track and hands them to `show` with their source's name, from the cache
/// or from the sources. Nothing is shown when none has any.
fn lookup(
    agent: &ureq::Agent,
    dir: &Path,
    track: &Track,
    providers: &[&'static str],
    show: impl Fn(Lines, &'static str),
) {
    let duration = track.duration.round() as u32;
    let file = cache_file(dir, track, duration);
    if let Some(entry) = read_entry(&file) {
        let source = if entry.source.is_empty() { "lrclib" } else { entry.source.as_str() };
        let fresh = now().saturating_sub(entry.at) < NOT_FOUND_TTL && entry.tried == providers;
        match entry.lines {
            Some(lines) if providers.contains(&source) && (fresh || has_words(&lines)) => {
                return show(lines, name(source));
            }
            None if fresh => return,
            _ => {}
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    for (rank, &provider) in providers.iter().enumerate() {
        let (tx, agent, track) = (tx.clone(), agent.clone(), track.clone());
        std::thread::spawn(move || {
            let _ = tx.send((rank, ask(&agent, provider, &track, duration)));
        });
    }
    drop(tx);
    // Each source's answer, once it comes.
    let mut answers: Vec<Option<Result<Option<Lines>, String>>> = vec![None; providers.len()];
    let mut shown = None;
    while let Ok((rank, answer)) = rx.recv() {
        if let Err(e) = &answer {
            eprintln!("lyrics: {}: {e}", providers[rank]);
        }
        answers[rank] = Some(answer);
        let Some(best) = best(&answers) else { continue };
        if shown != Some(best) {
            shown = Some(best);
            show(lines_of(&answers, best).expect("best has lines").clone(), name(providers[best]));
        }
        // Final once no source before it could still answer with word times.
        if answers[..best].iter().all(Option::is_some) && has_words(lines_of(&answers, best).expect("best")) {
            break;
        }
    }
    let tried = providers.iter().zip(&answers).filter(|(_, a)| matches!(a, Some(Ok(_)))).map(|(p, _)| p.to_string());
    let lines = shown.and_then(|b| lines_of(&answers, b).cloned());
    let source = shown.map_or(String::new(), |b| providers[b].to_string());
    write_entry(&file, &Entry { at: now(), lines, source, tried: tried.collect(), translations: Vec::new() });
}

/// The source that wins among the answers so far: the first with word times, else the first with lines.
fn best(answers: &[Option<Result<Option<Lines>, String>>]) -> Option<usize> {
    let with_lines = || (0..answers.len()).filter(|&i| lines_of(answers, i).is_some());
    with_lines().find(|&i| lines_of(answers, i).is_some_and(|l| has_words(l))).or_else(|| with_lines().next())
}

fn lines_of(answers: &[Option<Result<Option<Lines>, String>>], i: usize) -> Option<&Lines> {
    match &answers[i] {
        Some(Ok(Some(lines))) => Some(lines),
        _ => None,
    }
}

fn has_words(lines: &[Line]) -> bool {
    lines.iter().any(|l| !l.words.is_empty())
}

fn ask(agent: &ureq::Agent, provider: &str, track: &Track, duration: u32) -> Result<Option<Lines>, String> {
    match provider {
        "lrclib" => lrclib(agent, track, duration),
        "betterlyrics" => better_lyrics(agent, track, duration),
        "unison" => unison(agent, track, duration),
        _ => lrcmux(agent, track, duration),
    }
}

/// The answer's JSON; `None` for a 404 (or a 401, Better Lyrics' "not cached").
fn get_json(request: ureq::RequestBuilder<ureq::typestate::WithoutBody>) -> Result<Option<Value>, String> {
    match request.call() {
        Ok(mut response) => {
            let bytes = response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("unreadable answer: {e}"))
        }
        Err(ureq::Error::StatusCode(401 | 404)) => Ok(None),
        Err(e) => Err(format!("request failed: {e}")),
    }
}

fn lrclib(agent: &ureq::Agent, track: &Track, duration: u32) -> Result<Option<Lines>, String> {
    let mut request = agent
        .get(format!("{LRCLIB}/get"))
        .query("track_name", &track.title)
        .query("artist_name", &track.artist)
        .query("duration", duration.to_string());
    if !track.album.is_empty() {
        request = request.query("album_name", &track.album);
    }
    if let Some(lines) = get_json(request)?.as_ref().and_then(synced_lrc) {
        return Ok(Some(lines));
    }
    // No exact entry with synced lyrics: search with the cleaned title.
    let title = clean_title(&track.title, &track.artist);
    let artist = artists(&track.artist).into_iter().next().unwrap_or_default();
    let search = |query: &[(&str, &str)]| -> Result<Vec<Value>, String> {
        let mut request = agent.get(format!("{LRCLIB}/search"));
        for (k, v) in query {
            request = request.query(*k, *v);
        }
        Ok(get_json(request)?.and_then(|v| v.as_array().cloned()).unwrap_or_default())
    };
    let mut results = search(&[("track_name", &title), ("artist_name", &artist)])?;
    if results.is_empty() {
        results = search(&[("q", &format!("{artist} {title}"))])?;
    }
    Ok(best_match(&results, &title, &track.artist, track.duration))
}

fn synced_lrc(entry: &Value) -> Option<Lines> {
    if entry.get("instrumental").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    entry.get("syncedLyrics").and_then(Value::as_str).map(parse_lrc).filter(|l| !l.is_empty())
}

/// The search result closest in length among those whose artist and title match.
fn best_match(results: &[Value], title: &str, artist: &str, duration: f64) -> Option<Lines> {
    let title = normalize(title);
    let wanted = artists(artist);
    results
        .iter()
        .take(MAX_RESULTS)
        .filter_map(|r| {
            let gap = (r.get("duration")?.as_f64()? - duration).abs();
            let name = normalize(&clean_title(r.get("trackName")?.as_str()?, ""));
            let theirs = artists(r.get("artistName")?.as_str()?);
            let ok = gap <= MAX_DURATION_GAP
                && alike(&name, &title)
                && wanted.iter().any(|a| theirs.iter().any(|b| alike(a, b)));
            ok.then_some((gap, r))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .and_then(|(_, r)| synced_lrc(r))
}

/// Equal, or one holds the other (both normalized; not for very short names).
fn alike(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 3 && (a.contains(b) || b.contains(a)))
}

/// Lowercase letters and digits, words separated by one space.
fn normalize(s: &str) -> String {
    let mapped: String = s.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    mapped.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The artists of a credit ("A & B", "A, B", "A feat. B", "A x B"), normalized.
fn artists(credit: &str) -> Vec<String> {
    let n = format!(" {} ", normalize(credit));
    let mut parts = vec![n];
    for sep in [" feat ", " ft ", " featuring ", " x ", " with ", " and ", " e ", " y "] {
        parts = parts.iter().flat_map(|p| p.split(sep).map(String::from).collect::<Vec<_>>()).collect();
    }
    let split: Vec<String> = credit.split(['&', ',', '/']).map(normalize).filter(|a| !a.is_empty()).collect();
    let mut out: Vec<String> = parts.into_iter().map(|p| p.trim().to_owned()).filter(|a| !a.is_empty()).collect();
    for a in split {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// The title without what video titles add: bracketed parts ("(Official Video)", "[Lyrics]"), a
/// "feat." credit, an "Artist - " in front and a " - Remastered 2011" behind. The original if nothing
/// would be left.
fn clean_title(title: &str, artist: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' | '【' => depth += 1,
            ')' | ']' | '】' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let lower = out.to_lowercase();
    for mark in [" feat. ", " ft. ", " feat ", " featuring "] {
        if let Some(i) = lower.find(mark) {
            out.truncate(i);
            break;
        }
    }
    if let Some((before, after)) = out.split_once(" - ") {
        let b = normalize(before);
        if !artist.is_empty() && artists(artist).iter().any(|a| alike(a, &b)) {
            out = after.to_owned();
        } else if ["remaster", "version", "edit", "mono", "stereo"].iter().any(|w| after.to_lowercase().contains(w)) {
            out = before.to_owned();
        }
    }
    let out = out.trim().trim_matches(['-', '–', '—', '|']).trim();
    if out.is_empty() { title.trim().to_owned() } else { out.to_owned() }
}

fn better_lyrics(agent: &ureq::Agent, track: &Track, duration: u32) -> Result<Option<Lines>, String> {
    let mut request =
        agent.get(BETTER_LYRICS).query("s", &track.title).query("a", &track.artist).query("d", duration.to_string());
    if !track.album.is_empty() {
        request = request.query("al", &track.album);
    }
    let Some(ttml) = get_json(request)?.and_then(|v| v.get("ttml").and_then(Value::as_str).map(String::from)) else {
        return Ok(None);
    };
    // The whole song's length, when given: a different length is another recording.
    if let Some(dur) = attr(&ttml[ttml.find("<body").unwrap_or(0)..], "dur").and_then(parse_clock)
        && (f64::from(dur) / 1000.0 - track.duration).abs() > MAX_DURATION_GAP
    {
        return Ok(None);
    }
    let lines = parse_ttml(&ttml);
    Ok((!lines.is_empty()).then_some(lines))
}

fn unison(agent: &ureq::Agent, track: &Track, duration: u32) -> Result<Option<Lines>, String> {
    let mut body = None;
    if let Some(id) = &track.id {
        body = get_json(agent.get(UNISON).query("v", id))?;
    }
    if body.is_none() {
        let request = agent
            .get(UNISON)
            .query("song", &track.title)
            .query("artist", &track.artist)
            .query("duration", duration.to_string());
        body = get_json(request)?;
    }
    let Some(data) = body.as_ref().and_then(|b| b.get("data")) else { return Ok(None) };
    if data.get("hidden").and_then(Value::as_bool) == Some(true)
        || data.get("duration").and_then(Value::as_f64).is_some_and(|d| (d - track.duration).abs() > MAX_DURATION_GAP)
    {
        return Ok(None);
    }
    let text = data.get("lyrics").and_then(Value::as_str).unwrap_or_default();
    let lines = match data.get("format").and_then(Value::as_str) {
        Some("ttml") => parse_ttml(text),
        Some("lrc") => parse_lrc(text),
        _ => Vec::new(),
    };
    Ok((!lines.is_empty()).then_some(lines))
}

fn lrcmux(agent: &ureq::Agent, track: &Track, duration: u32) -> Result<Option<Lines>, String> {
    let mut request = agent
        .get(LRCMUX)
        .query("artist", &track.artist)
        .query("title", &track.title)
        .query("duration", duration.to_string())
        .query("level", "word")
        .query("strict", "true")
        .query("sources", "!ytmusic,!genius");
    if !track.album.is_empty() {
        request = request.query("album", &track.album);
    }
    let Some(body) = get_json(request)? else { return Ok(None) };
    if body
        .pointer("/track/duration")
        .and_then(Value::as_f64)
        .is_some_and(|d| (d - track.duration).abs() > MAX_DURATION_GAP)
    {
        return Ok(None);
    }
    let start = |v: &Value| v.get("start").and_then(Value::as_u64).map(|ms| ms.min(u64::from(u32::MAX)) as u32);
    let mut lines: Lines = Vec::new();
    for line in body.get("lines").and_then(Value::as_array).into_iter().flatten().take(MAX_LINES) {
        let (Some(at), Some(text)) = (start(line), line.get("text").and_then(Value::as_str)) else { continue };
        if text.contains('\u{FFFD}') {
            return Ok(None);
        }
        let words: Vec<(u32, String)> = line
            .get("words")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|w| Some((start(w)?, w.get("text")?.as_str()?.to_owned())))
            .collect();
        let (text, words) =
            if words.is_empty() { make_words(vec![(at, text.to_owned())], false) } else { make_words(words, true) };
        if !text.is_empty() {
            lines.push(Line { ms: at, text, words, ..Default::default() });
        }
    }
    lines.sort_by_key(|l| l.ms);
    Ok((!lines.is_empty()).then_some(lines))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_file(dir: &Path, track: &Track, duration: u32) -> PathBuf {
    let text = format!("{}\0{}\0{}\0{duration}", track.title, track.artist, track.album).to_lowercase();
    dir.join(format!("{:x}.json", md5::compute(text)))
}

fn read_entry(file: &Path) -> Option<Entry> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

fn write_entry(file: &Path, entry: &Entry) {
    let Some(dir) = file.parent() else { return };
    let _ = std::fs::create_dir_all(dir);
    match serde_json::to_vec(entry) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(file, bytes) {
                eprintln!("lyrics: cache not written: {e}");
            }
        }
        Err(e) => eprintln!("lyrics: cache not written: {e}"),
    }
    prune(dir);
}

/// Keeps the cache small: over `MAX_FILES`, the oldest go.
fn prune(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> =
        read.filter_map(Result::ok).filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= MAX_FILES {
        return;
    }
    files.sort();
    for (_, path) in &files[..files.len() - KEEP_FILES] {
        let _ = std::fs::remove_file(path);
    }
}

/// `[mm:ss.xx] text` lines (several stamps per line allowed) into lines by time, in milliseconds.
/// Metadata tags (`[ar:...]`) and lines without a time are dropped; the text is capped.
fn parse_lrc(text: &str) -> Lines {
    let mut lines: Lines = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim_start();
        let mut times = Vec::new();
        while let Some(tagged) = rest.strip_prefix('[') {
            let Some((tag, after)) = tagged.split_once(']') else { break };
            let Some(ms) = parse_time(tag) else { break };
            times.push(ms);
            rest = after;
        }
        let (text, words) = parse_words(rest);
        for ms in times {
            lines.push(Line { ms, text: text.clone(), words: words.clone(), ..Default::default() });
        }
    }
    lines.sort_by_key(|l| l.ms);
    lines.truncate(MAX_LINES);
    lines
}

/// The text of a line without its word stamps, and its words when it has them (Enhanced LRC:
/// `<mm:ss.xx>word <mm:ss.xx>word`). Text before the first stamp is sung with the line.
fn parse_words(rest: &str) -> (String, Words) {
    let mut segments = Vec::new();
    let mut at = 0;
    let mut s = rest;
    loop {
        let next = stamp(s);
        segments.push((at, next.map_or(s, |(start, _, _)| &s[..start]).to_owned()));
        let Some((_, end, ms)) = next else { break };
        at = ms;
        s = &s[end..];
    }
    let stamped = segments.len() > 1;
    make_words(segments, stamped)
}

/// A line from timed segments (`(ms, text)`; the first may be text sung with the line): its text, and
/// its words when `timed`. Control characters go, the text is capped, and the words' text adds up to
/// the line's.
fn make_words(segments: Vec<(u32, String)>, timed: bool) -> (String, Words) {
    let clean = |s: &str| -> String { s.chars().filter(|c| !c.is_control()).collect() };
    let mut plain = String::new();
    let mut words: Words = Vec::new();
    for (at, segment) in segments {
        let segment = clean(&segment);
        plain.push_str(&segment);
        if !segment.trim().is_empty() && words.len() < MAX_WORDS {
            words.push((at, segment));
        } else if let Some(last) = words.last_mut() {
            last.1.push_str(&segment);
        }
    }
    let text: String = plain.trim().chars().take(MAX_LINE_CHARS).collect();
    if !timed || text.is_empty() {
        return (text, Vec::new());
    }
    if let Some(first) = words.first_mut() {
        first.1 = first.1.trim_start().to_owned();
    }
    if let Some(last) = words.last_mut() {
        last.1 = last.1.trim_end().to_owned();
    }
    for word in &mut words {
        word.1 = word.1.chars().take(MAX_LINE_CHARS).collect();
    }
    (text, words)
}

/// The first `<mm:ss.xx>` in `s`: where it starts, where it ends and its time.
fn stamp(s: &str) -> Option<(usize, usize, u32)> {
    s.match_indices('<').find_map(|(start, _)| {
        let len = s[start + 1..].find('>')?;
        let ms = parse_time(&s[start + 1..start + 1 + len])?;
        Some((start, start + len + 2, ms))
    })
}

/// `mm:ss` or `mm:ss.xx`, in milliseconds.
fn parse_time(tag: &str) -> Option<u32> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u32 = minutes.parse().ok().filter(|m| *m < 600)?;
    let seconds: f64 = seconds.parse().ok().filter(|s| (0.0..60.0).contains(s))?;
    Some(minutes * 60_000 + (seconds * 1000.0).round() as u32)
}

/// A TTML clock time (`12.345`, `1:02.5`, `1:02:03.4`, `12.3s`), in milliseconds.
fn parse_clock(text: &str) -> Option<u32> {
    let text = text.trim().trim_end_matches('s');
    let mut seconds = 0.0;
    for (i, part) in text.split(':').enumerate() {
        if i > 2 {
            return None;
        }
        let v: f64 = part.parse().ok().filter(|v: &f64| v.is_finite() && *v >= 0.0)?;
        seconds = seconds * 60.0 + v;
    }
    (seconds < 36_000.0).then(|| (seconds * 1000.0).round() as u32)
}

/// The value of `name="..."` in the first tag of `tag`.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let head = &tag[..tag.find('>')?];
    let at = head
        .match_indices(name)
        .find(|(i, _)| head[..*i].ends_with(char::is_whitespace) && head[i + name.len()..].starts_with("=\""))?;
    let rest = &head[at.0 + name.len() + 2..];
    Some(&rest[..rest.find('"')?])
}

/// `&amp;` and the other XML escapes.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let c = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .unwrap_or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// TTML lyrics into lines: each `<p begin>` is a line, each timed `<span begin>` in it a word (the text
/// between spans, usually a space, goes with the word before). Background vocals
/// (`<span ttm:role="x-bg">`) are left out. Translations in its head (`<translation xml:lang>` with a
/// `<text for="L1">` per line `<p itunes:key="L1">`) go to the lines' `translations`.
fn parse_ttml(xml: &str) -> Lines {
    let mut lines: Lines = Vec::new();
    // The key of each line, for its translations.
    let mut keys: Vec<Option<&str>> = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<p ").or_else(|| rest.find("<p>")) {
        rest = &rest[i..];
        let Some(open_end) = rest.find('>') else { break };
        let Some(close) = rest.find("</p>") else { break };
        let begin = attr(rest, "begin").and_then(parse_clock);
        let key = attr(rest, "itunes:key").or_else(|| attr(rest, "lrc:key"));
        let inner = &rest[open_end + 1..close.max(open_end + 1)];
        rest = &rest[close + 4..];
        let Some(begin) = begin else { continue };
        // (time, text) per word; the first holds the text before any timed span.
        let mut segments: Vec<(u32, String)> = vec![(begin, String::new())];
        // Each open span: whether it is (inside) background vocals.
        let mut spans: Vec<bool> = Vec::new();
        let mut s = inner;
        while !s.is_empty() {
            if let Some(tag) = s.strip_prefix('<') {
                let Some(end) = tag.find('>') else { break };
                let body = &tag[..end];
                if body.starts_with("/span") {
                    spans.pop();
                } else if body.starts_with("span") {
                    let bg = spans.last().copied().unwrap_or(false) || body.contains("x-bg");
                    if !bg && let Some(at) = attr(s, "begin").and_then(parse_clock) {
                        segments.push((at, String::new()));
                    }
                    if !body.ends_with('/') {
                        spans.push(bg);
                    }
                }
                s = &tag[end + 1..];
            } else {
                let end = s.find('<').unwrap_or(s.len());
                if !spans.last().copied().unwrap_or(false)
                    && let Some(last) = segments.last_mut()
                {
                    last.1.push_str(&unescape(&s[..end]));
                }
                s = &s[end..];
            }
        }
        let timed = segments.len() > 1;
        if timed && segments[0].1.trim().is_empty() {
            let lead = segments.remove(0).1;
            segments[0].1.insert_str(0, &lead);
        }
        let (text, words) = make_words(segments, timed);
        if !text.is_empty() {
            lines.push(Line { ms: begin, text, words, ..Default::default() });
            keys.push(key);
        }
        if lines.len() >= MAX_LINES {
            break;
        }
    }
    let mut languages = 0;
    let mut rest = xml;
    while let Some(i) = rest.find("<translation ") {
        rest = &rest[i..];
        let end = rest.find("</translation>").unwrap_or(rest.len());
        let block = &rest[..end];
        rest = &rest[end..];
        let Some(lang) = attr(block, "xml:lang").filter(|l| !l.is_empty() && l.len() <= 35) else { continue };
        let mut texts = block;
        while let Some(j) = texts.find("<text ") {
            texts = &texts[j..];
            let Some(open_end) = texts.find('>') else { break };
            let close = texts.find("</text>").unwrap_or(texts.len()).max(open_end + 1);
            let key = attr(texts, "for");
            let text = clean_text(&plain_text(&texts[open_end + 1..close]));
            texts = &texts[close..];
            if let Some(i) = key.and_then(|k| keys.iter().position(|l| *l == Some(k)))
                && !text.is_empty()
                && !text.eq_ignore_ascii_case(&lines[i].text)
            {
                lines[i].translations.push((lang.to_owned(), text));
            }
        }
        languages += 1;
        if languages >= MAX_TRANSLATIONS {
            break;
        }
    }
    lines.sort_by_key(|l| l.ms);
    lines
}

/// The text of TTML markup without its tags and without background vocals.
fn plain_text(markup: &str) -> String {
    let mut out = String::new();
    let mut spans: Vec<bool> = Vec::new();
    let mut s = markup;
    while !s.is_empty() {
        if let Some(tag) = s.strip_prefix('<') {
            let Some(end) = tag.find('>') else { break };
            let body = &tag[..end];
            if body.starts_with("/span") {
                spans.pop();
            } else if body.starts_with("span") && !body.ends_with('/') {
                spans.push(spans.last().copied().unwrap_or(false) || body.contains("x-bg"));
            }
            s = &tag[end + 1..];
        } else {
            let end = s.find('<').unwrap_or(s.len());
            if !spans.last().copied().unwrap_or(false) {
                out.push_str(&unescape(&s[..end]));
            }
            s = &s[end..];
        }
    }
    out
}

/// Moves every time by `offset` ms (positive: later), never before 0.
fn shift(lines: &mut [Line], offset: i32) {
    if offset == 0 {
        return;
    }
    let moved = |ms: u32| (i64::from(ms) + i64::from(offset)).clamp(0, i64::from(u32::MAX)) as u32;
    for line in lines {
        line.ms = moved(line.ms);
        for word in &mut line.words {
            word.0 = moved(word.0);
        }
    }
}

// Estimated word times, for lines timed only as a whole. Each word weighs its syllables (groups of
// vowels; +0.5 after a comma), and a line is sung at the song's own pace: the first quartile of time per
// syllable over its lines (lines sung back to back), so the words do not stretch over an instrumental
// gap before the next line; a line with time to spare takes `SLACK` of it, as slow lines do. Measured
// against real word times (7 songs, 2411 words): half the error of spreading the words over the whole
// line, and early as often as late (a weight for a held last word made one line in three light a word
// 0.4 s early; now one in five).
const RATE_DEFAULT_MS: f64 = 300.0;
const RATE_MIN_MS: f64 = 120.0;
const RATE_MAX_MS: f64 = 600.0;
/// A line is sung within this share of the time to the next one.
const SUNG_SHARE: f64 = 0.92;
/// The time after the last line.
const LAST_LINE_MS: f64 = 4000.0;
/// The share of a line's spare time (beyond its pace) that it takes.
const SLACK: f64 = 0.15;
const PAUSE: f64 = 0.5;

/// Adds estimated word times to every line, when no line has word times of its own. Returns whether
/// it did.
pub fn estimate(lines: &mut [Line]) -> bool {
    if lines.is_empty() || lines.iter().any(|l| !l.words.is_empty()) {
        return false;
    }
    let tokens: Vec<Vec<String>> = lines.iter().map(|l| split_words(&l.text)).collect();
    let weights: Vec<Vec<f64>> = tokens.iter().map(|t| weigh(t)).collect();
    let gaps: Vec<f64> = (0..lines.len())
        .map(|i| lines.get(i + 1).map_or(LAST_LINE_MS, |n| f64::from(n.ms.saturating_sub(lines[i].ms))))
        .collect();
    let gap = |i: usize| gaps[i];
    let mut rates: Vec<f64> = (0..lines.len().saturating_sub(1))
        .filter(|&i| !weights[i].is_empty())
        .map(|i| gap(i) / weights[i].iter().sum::<f64>())
        .filter(|r| *r > 0.0)
        .collect();
    rates.sort_by(f64::total_cmp);
    let rate = rates.get(rates.len() / 4).copied().unwrap_or(RATE_DEFAULT_MS).clamp(RATE_MIN_MS, RATE_MAX_MS);
    let mut any = false;
    for (i, (words, weights)) in tokens.into_iter().zip(weights).enumerate() {
        if words.len() < 2 || words.len() > MAX_WORDS {
            continue;
        }
        let total: f64 = weights.iter().sum();
        let room = gap(i) * SUNG_SHARE;
        let paced = total * rate;
        let sung = if room > paced { paced + (room - paced) * SLACK } else { room };
        let start = f64::from(lines[i].ms);
        let mut before = 0.0;
        lines[i].words = words
            .into_iter()
            .zip(&weights)
            .map(|(word, w)| {
                let at = start + sung * before / total;
                before += w;
                (at.round() as u32, word)
            })
            .collect();
        any = true;
    }
    any
}

/// The words of a line, each with the spaces after it (together they are the line). Chinese and
/// Japanese characters are a word each.
fn split_words(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut spaced = false;
    for c in text.chars() {
        let new_word = match words.last() {
            None => true,
            Some(_) if c.is_whitespace() => false,
            Some(_) => spaced || cjk(c) || words.last().and_then(|w| w.chars().last()).is_some_and(cjk),
        };
        if new_word {
            words.push(String::new());
        }
        words.last_mut().expect("pushed").push(c);
        spaced = c.is_whitespace();
    }
    words
}

fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

fn weigh(words: &[String]) -> Vec<f64> {
    words.iter().map(|w| syllables(w) + if w.trim_end().ends_with([',', ';', ':']) { PAUSE } else { 0.0 }).collect()
}

/// Syllables of a word: groups of vowels (Latin with accents, Cyrillic), Hangul blocks, else a guess
/// from its length; at least 1. A silent final "e" (English) is not counted.
fn syllables(word: &str) -> f64 {
    const VOWELS: &str = "aeiouyáéíóúàèìòùâêîôûãõäëïöüåæøœаеёиоуыэюя";
    let w: String = word.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let mut groups = 0;
    let mut in_vowel = false;
    let mut hangul = 0;
    for c in w.chars() {
        let v = VOWELS.contains(c);
        if v && !in_vowel {
            groups += 1;
        }
        in_vowel = v;
        if ('\u{AC00}'..='\u{D7A3}').contains(&c) {
            hangul += 1;
        }
    }
    if groups > 1 && w.ends_with('e') && !w.ends_with("le") && !w.ends_with("ee") {
        groups -= 1;
    }
    let guess = w.chars().filter(|c| c.is_alphanumeric()).count() / 3;
    groups.max(hangul).max(if groups == 0 && hangul == 0 { guess } else { 0 }).max(1) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttml_words_skip_background_vocals() {
        let xml = r#"<tt><body dur="2:33.181"><div><p begin="0.000" end="1.5" ttm:agent="v1"><span begin="0.000" end="0.451">I</span> <span begin="0.451" end="1.539">lo&amp;ve</span><span ttm:role="x-bg"><span begin="1.0" end="1.4">(echo)</span></span></p><p begin="1:02.5" end="1:04">Line only</p></div></body></tt>"#;
        let lines = parse_ttml(xml);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "I lo&ve");
        assert_eq!(lines[0].words, vec![(0, "I ".to_owned()), (451, "lo&ve".to_owned())]);
        assert_eq!((lines[1].ms, lines[1].text.as_str(), lines[1].words.len()), (62_500, "Line only", 0));
        assert_eq!(attr(&xml[xml.find("<body").unwrap()..], "dur").and_then(parse_clock), Some(153_181));
    }

    #[test]
    fn video_titles_are_cleaned() {
        assert_eq!(clean_title("Artist - Song (Official Video) [4K]", "Artist"), "Song");
        assert_eq!(clean_title("Song feat. Other", "Artist"), "Song");
        assert_eq!(clean_title("Song - Remastered 2011", "Artist"), "Song");
        assert_eq!(clean_title("(Intro)", "Artist"), "(Intro)");
        assert!(artists("A & B feat. C").contains(&"c".to_owned()));
    }

    #[test]
    fn estimated_words_add_up_to_the_line_and_stay_in_it() {
        let mut lines = vec![
            Line { ms: 1000, text: "Hello there, my old friend".into(), ..Default::default() },
            Line { ms: 4000, text: "你好世界".into(), ..Default::default() },
            Line { ms: 30_000, text: "Solo".into(), ..Default::default() },
        ];
        assert!(estimate(&mut lines));
        let joined: String = lines[0].words.iter().map(|w| w.1.as_str()).collect();
        assert_eq!(joined, lines[0].text);
        assert_eq!(lines[0].words[0].0, 1000);
        assert!(lines[0].words.windows(2).all(|w| w[0].0 < w[1].0) && lines[0].words.last().unwrap().0 < 4000);
        assert_eq!(lines[1].words.len(), 4);
        assert!(lines[2].words.is_empty());
        assert!(!estimate(&mut lines));
    }

    type Answer = Result<Option<Lines>, String>;

    /// The `p`th percentile (0..=1) of sorted values.
    fn percentile(sorted: &[u64], p: f64) -> u64 {
        sorted.get(((sorted.len().max(1) - 1) as f64 * p).round() as usize).copied().unwrap_or(0)
    }

    fn median_p90(mut ms: Vec<u64>) -> String {
        ms.sort_unstable();
        format!("{:>7} {:>7}", percentile(&ms, 0.5), percentile(&ms, 0.9))
    }

    /// How often each source finds a track, how often with word times, and how fast, over a list of
    /// tracks; then the same for the combined lookup (first answer shown, last change of the lines).
    /// It asks the real services (one track every 2 s per source, at most 100 tracks), so it is run
    /// by hand; the file has one track per line: title, artist, album, duration in seconds, video id
    /// (the id may be empty), tab-separated.
    ///   FEATHERTUNE_BENCH_TRACKS=<file> cargo test --release -- --ignored sources_bench --nocapture
    #[test]
    #[ignore]
    fn sources_bench() {
        use std::time::Instant;

        let path = std::env::var("FEATHERTUNE_BENCH_TRACKS").expect("FEATHERTUNE_BENCH_TRACKS: the track list");
        let tracks: Vec<Track> = std::fs::read_to_string(path)
            .expect("track list")
            .lines()
            .filter_map(|line| {
                let f: Vec<&str> = line.split('\t').map(str::trim).collect();
                Some(Track {
                    id: f.get(4).filter(|id| !id.is_empty()).map(|id| id.to_string()),
                    title: f.first()?.to_string(),
                    artist: f.get(1)?.to_string(),
                    album: f.get(2)?.to_string(),
                    artwork: None,
                    duration: f.get(3)?.parse().ok()?,
                })
            })
            .take(100)
            .collect();
        assert!(!tracks.is_empty(), "no tracks in the list");

        let agent = agent();
        // One thread per source, each at most one track every 2 s; no cache involved.
        let threads: Vec<_> = PROVIDERS
            .iter()
            .map(|&(id, _)| {
                let (agent, tracks) = (agent.clone(), tracks.clone());
                std::thread::spawn(move || {
                    let mut last: Option<Instant> = None;
                    let mut out: Vec<(u64, Answer)> = Vec::new();
                    for track in &tracks {
                        if let Some(l) = last {
                            std::thread::sleep(Duration::from_secs(2).saturating_sub(l.elapsed()));
                        }
                        let t = Instant::now();
                        last = Some(t);
                        let answer = ask(&agent, id, track, track.duration.round() as u32);
                        out.push((t.elapsed().as_millis() as u64, answer));
                    }
                    out
                })
            })
            .collect();
        let results: Vec<Vec<(u64, Answer)>> = threads.into_iter().map(|t| t.join().unwrap()).collect();

        println!("\n{} tracks", tracks.len());
        println!(
            "{:<13} {:>6} {:>7} {:>8} {:>7}   {:>7} {:>7}",
            "source", "errors", "found%", "words%", "answered", "med ms", "p90 ms"
        );
        for ((id, _), rows) in PROVIDERS.iter().zip(&results) {
            let answered = rows.iter().filter(|(_, a)| a.is_ok()).count();
            let found = rows.iter().filter(|(_, a)| matches!(a, Ok(Some(_)))).count();
            let words = rows.iter().filter(|(_, a)| matches!(a, Ok(Some(l)) if has_words(l))).count();
            let pct = |n: usize| 100.0 * n as f64 / answered.max(1) as f64;
            println!(
                "{:<13} {:>6} {:>6.0}% {:>7.0}% {:>8}   {}",
                id,
                rows.len() - answered,
                pct(found),
                pct(words),
                answered,
                median_p90(rows.iter().map(|r| r.0).collect()),
            );
        }

        // The combined lookup, replayed from the same answers in the order they came.
        let (mut first, mut last, mut found, mut words) = (Vec::new(), Vec::new(), 0, 0);
        let per_track = (0..tracks.len()).map(|i| results.iter().map(|rows| &rows[i]).collect::<Vec<_>>());
        for sources in per_track {
            let mut order: Vec<usize> = (0..PROVIDERS.len()).collect();
            order.sort_by_key(|&r| sources[r].0);
            let mut answers: Vec<Option<Answer>> = vec![None; PROVIDERS.len()];
            let (mut shown, mut first_ms, mut last_ms) = (None, None, 0);
            for r in order {
                answers[r] = Some(sources[r].1.clone());
                let Some(best) = best(&answers) else { continue };
                if shown != Some(best) {
                    shown = Some(best);
                    first_ms.get_or_insert(sources[r].0);
                    last_ms = sources[r].0;
                }
                if answers[..best].iter().all(Option::is_some) && has_words(lines_of(&answers, best).unwrap()) {
                    break;
                }
            }
            if let (Some(best), Some(ms)) = (shown, first_ms) {
                found += 1;
                words += usize::from(has_words(lines_of(&answers, best).unwrap()));
                first.push(ms);
                last.push(last_ms);
            }
        }
        let pct = |n: usize| 100.0 * n as f64 / tracks.len() as f64;
        println!("\ncombined lookup: found {:.0}%, with word times {:.0}%", pct(found), pct(words));
        println!("  first answer  med / p90 ms: {}", median_p90(first));
        println!("  final answer  med / p90 ms: {}", median_p90(last));
    }
}
