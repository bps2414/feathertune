//! Tracker filter (plugin `tracker_filter`, on by default: `[plugins] tracker_filter = false` in
//! `config.toml` turns it off).
//!
//! Third-party requests that match a *privacy* list (analytics, session recording) are answered
//! with an empty 403 from WebView2's `WebResourceRequested`, so they never reach the network. The
//! list is parsed by the `adblock` crate, with cosmetic rules dropped: the page is never changed.
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
//! COM notes: the handler runs on the UI thread (the `adblock` engine is single-threaded and is
//! built there), and the filters are added after the window exists, so a request made before that
//! first main-thread turn is not seen. The site's own startup requests are first-party anyway.

use std::collections::HashSet;
use std::path::Path;

use adblock::lists::{ParseOptions, RuleTypes};
use adblock::request::Request;
use adblock::{Engine, FilterSet};
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
/// The page that makes the requests, for the list's `$third-party` test.
const SOURCE: &str = "https://music.youtube.com/";

/// The only resource kinds the handler is ever called for, with the request type the list sees.
const CONTEXTS: &[(COREWEBVIEW2_WEB_RESOURCE_CONTEXT, &str)] = &[
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT, "script"),
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, "image"),
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST, "xhr"),
    // The list syntax has no separate type for fetch(); it counts as XHR.
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH, "xhr"),
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_PING, "ping"),
    (COREWEBVIEW2_WEB_RESOURCE_CONTEXT_OTHER, "other"),
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
    let mut set = FilterSet::new(false);
    set.add_filter_list(rules.to_owned(), ParseOptions { rule_types: RuleTypes::NetworkOnly, ..Default::default() });
    let engine = Engine::new_with_filter_set(set);
    let environment = wv.environment();
    let core = unsafe { wv.controller().CoreWebView2()? };
    for (context, _) in CONTEXTS {
        unsafe { core.AddWebResourceRequestedFilter(w!("*"), *context)? };
    }

    let mut logged = HashSet::new();
    let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
        unsafe { args.ResourceContext(&mut context)? };
        let Some((_, kind)) = CONTEXTS.iter().find(|(c, _)| *c == context) else { return Ok(()) };
        let mut uri = PWSTR::null();
        unsafe { args.Request()?.Uri(&mut uri)? };
        let uri = take_pwstr(uri);
        let Some(host) = filterable_host(&uri) else { return Ok(()) };
        let blocked = Request::new(&uri, SOURCE, kind, "GET")
            .map(|request| engine.check_network_request(&request).should_block())
            .unwrap_or(false);
        if blocked {
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
        let mut set = FilterSet::new(false);
        set.add_filter_list(
            BUNDLED_LIST.to_owned(),
            ParseOptions { rule_types: RuleTypes::NetworkOnly, ..Default::default() },
        );
        let engine = Engine::new_with_filter_set(set);
        let check = |url: &str, kind: &str| {
            engine.check_network_request(&Request::new(url, SOURCE, kind, "GET").unwrap()).should_block()
        };
        assert!(check("https://www.google-analytics.com/g/collect?v=2", "xhr"));
        assert!(check("https://static.hotjar.com/c/hotjar-1.js", "script"));
        assert!(!check("https://music.youtube.com/youtubei/v1/next", "xhr"));
        assert!(!check("https://i.ytimg.com/vi/x/hq.jpg", "image"));
    }
}
