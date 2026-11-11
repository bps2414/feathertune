//! Tracker filter (plugin `tracker_filter`, on by default: `[plugins] tracker_filter = false` in
//! `config.toml` turns it off).
//!
//! Third-party requests that match a *privacy* list (analytics, session recording) are answered
//! with an empty 403 from WebView2's `WebResourceRequested`, so they never reach the network. Only
//! whole-host rules (`||host^`) are read (`blocked_hosts`): the page is never changed.
//!
//! What it must never do, and how that is enforced in code and not left to the list:
//! - Ads are not touched. Google's ad-delivery hosts and everything on a Google or player domain
//!   are skipped (`exempt`), so even an ad list dropped in by mistake changes nothing there.
//! - What the player plays is not touched. Only scripts, images, XHR/fetch, pings and "other"
//!   subresources reach the handler (the filters below), never documents, frames, media, fonts or
//!   stylesheets.
//!
//! Lists: `filters/privacy.txt` (compiled in) plus, if present, `privacy-list.txt` in the profile.
//! Under `--measure` every blocked host is printed once, so a run shows what the filter caught.
//!
//! COM notes: the handler runs on the UI thread (the host set is built there), and the filters are
//! added after the window exists, so a request made before that
//! first main-thread turn is not seen. The site's own startup requests are first-party anyway.

use std::collections::HashSet;
use std::path::Path;

use tauri::webview::PlatformWebview;
use tauri::{Url, Webview};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_OTHER,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
};
use webview2_com::{WebResourceRequestedEventHandler, take_pwstr};
use windows::Win32::System::Com::IStream;
use windows_core::{PWSTR, w};

const BUNDLED_LIST: &str = include_str!("../filters/privacy.txt");

/// The only resource kinds the handler is ever called for.
const CONTEXTS: &[COREWEBVIEW2_WEB_RESOURCE_CONTEXT] = &[
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_OTHER,
];

/// Registrable domains that are never filtered: the player, its media and images, Google's
/// infrastructure and Google's ad delivery. A host matches if it is the domain or a subdomain.
const EXEMPT_DOMAINS: &[&str] = &[
    "youtube.com",
    "youtube-nocookie.com",
    "youtu.be",
    "googlevideo.com",
    "ytimg.com",
    "ggpht.com",
    "googleusercontent.com",
    "gstatic.com",
    "googleapis.com",
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "googletagservices.com",
    "2mdn.net",
];

/// Adds the filter to the site webview. Does nothing but log if the list or COM fails.
pub fn install(webview: &Webview, profile: &Path, log_blocked: bool) {
    let mut rules = String::from(BUNDLED_LIST);
    if let Ok(extra) = std::fs::read_to_string(profile.join("privacy-list.txt")) {
        rules.push('\n');
        rules.push_str(&extra);
    }
    let _ = webview.with_webview(move |wv| {
        if let Err(e) = unsafe { attach(&wv, &rules, log_blocked) } {
            eprintln!("tracker filter unavailable: {e}");
        }
    });
}

unsafe fn attach(wv: &PlatformWebview, rules: &str, log_blocked: bool) -> windows_core::Result<()> {
    let hosts = blocked_hosts(rules);
    let environment = wv.environment();
    let core = unsafe { wv.controller().CoreWebView2()? };
    for context in CONTEXTS {
        unsafe { core.AddWebResourceRequestedFilter(w!("*"), *context)? };
    }

    let mut logged = HashSet::new();
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
        unsafe { args.ResourceContext(&mut context)? };
        if !CONTEXTS.contains(&context) {
            return Ok(());
        }
        let mut uri = PWSTR::null();
        unsafe { args.Request()?.Uri(&mut uri)? };
        let uri = take_pwstr(uri);
        let Some(host) = filterable_host(&uri) else { return Ok(()) };
        if listed(&hosts, &host) {
            let response =
                unsafe { environment.CreateWebResourceResponse(None::<&IStream>, 403, w!("Blocked"), w!(""))? };
            unsafe { args.SetResponse(&response)? };
            if log_blocked && logged.insert(host.clone()) {
                println!("tracker_blocked={host}");
            }
        }
        Ok(())
    }));
    let mut token = 0;
    unsafe { core.add_WebResourceRequested(&handler, &mut token) }
}

/// The hosts the lists block: their `||host^` rules, with no option or only `$third-party` (every host
/// that may be filtered is third-party to the player). Any other rule is ignored, and a host named by
/// an exception (`@@||host`) is never blocked.
fn blocked_hosts(rules: &str) -> HashSet<String> {
    let host_of = |pattern: &str| {
        let end = pattern.find(['^', '/', ':', '*', '|']).unwrap_or(pattern.len());
        let (host, rest) = pattern.split_at(end);
        let host = host.to_ascii_lowercase();
        let valid = !host.is_empty()
            && host.contains('.')
            && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        valid.then_some((host, rest.is_empty() || rest == "^"))
    };
    let (mut blocked, mut allowed) = (HashSet::new(), HashSet::new());
    for line in rules.lines().map(str::trim) {
        if let Some(rule) = line.strip_prefix("@@||") {
            if let Some((host, _)) = host_of(rule.split('$').next().unwrap_or("")) {
                allowed.insert(host);
            }
        } else if let Some(rule) = line.strip_prefix("||") {
            let (pattern, options) = rule.split_once('$').unwrap_or((rule, ""));
            let plain = options.split(',').all(|o| matches!(o, "" | "third-party" | "3p"));
            if let Some((host, true)) = host_of(pattern).filter(|_| plain) {
                blocked.insert(host);
            }
        }
    }
    blocked.retain(|h| !allowed.contains(h));
    blocked
}

/// Whether `host` or a domain above it is in `hosts`.
fn listed(hosts: &HashSet<String>, host: &str) -> bool {
    let mut h = host;
    loop {
        if hosts.contains(h) {
            return true;
        }
        match h.split_once('.') {
            Some((_, up)) => h = up,
            None => return false,
        }
    }
}

/// The host of an http(s) URL that is allowed to be filtered, `None` for everything else.
fn filterable_host(uri: &str) -> Option<String> {
    let url = Url::parse(uri).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let host = url.host_str()?.to_ascii_lowercase();
    (!exempt(&host)).then_some(host)
}

/// Player, Google and ad-delivery hosts. Any `google` label covers `google.com`, `google.co.uk`,
/// `adservice.google.com.br` and so on.
fn exempt(host: &str) -> bool {
    host.split('.').any(|label| label == "google")
        || EXEMPT_DOMAINS.iter().any(|d| host == *d || host.strip_suffix(d).is_some_and(|rest| rest.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempts_player_google_and_ad_hosts() {
        for host in [
            "music.youtube.com",
            "rr1---sn-abc.googlevideo.com",
            "i.ytimg.com",
            "lh3.googleusercontent.com",
            "accounts.google.com",
            "adservice.google.com.br",
            "securepubads.g.doubleclick.net",
            "pagead2.googlesyndication.com",
            "static.doubleclick.net",
        ] {
            assert!(exempt(host), "{host}");
        }
    }

    #[test]
    fn filters_other_hosts_only() {
        assert!(!exempt("www.google-analytics.com"));
        assert!(!exempt("notyoutube.com"));
        assert!(!exempt("evilgooglevideo.com"));
        assert_eq!(filterable_host("https://cdn.hotjar.com/a.js").as_deref(), Some("cdn.hotjar.com"));
        assert_eq!(filterable_host("https://music.youtube.com/youtubei/v1/next"), None);
        assert_eq!(filterable_host("wss://cdn.hotjar.com/socket"), None);
        assert_eq!(filterable_host("not a url"), None);
    }

    #[test]
    fn bundled_list_blocks_trackers_but_not_the_site() {
        let hosts = blocked_hosts(BUNDLED_LIST);
        assert_eq!(hosts.len(), 10);
        let check = |url: &str| filterable_host(url).is_some_and(|h| listed(&hosts, &h));
        assert!(check("https://www.google-analytics.com/g/collect?v=2"));
        assert!(check("https://static.hotjar.com/c/hotjar-1.js"));
        assert!(!check("https://notgoogle-analytics.com/x.js"));
        assert!(!check("https://music.youtube.com/youtubei/v1/next"));
        assert!(!check("https://i.ytimg.com/vi/x/hq.jpg"));
    }

    #[test]
    fn only_whole_host_rules_count() {
        let hosts = blocked_hosts(
            "! comment\n[Adblock Plus 2.0]\n||a.com^\n||B.com\n||c.com^$third-party\n||d.com^$script\n\
             ||e.com/path\n/tracker.js\n##.banner\n||f.com^\n@@||f.com^$xhr\n||bad host^\n||nodot^",
        );
        let mut got: Vec<_> = hosts.into_iter().collect();
        got.sort();
        assert_eq!(got, ["a.com", "b.com", "c.com"]);
    }
}
