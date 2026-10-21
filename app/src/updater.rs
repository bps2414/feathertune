//! Self-update for the launcher (splash.rs): asks GitHub for the latest release, downloads its
//! installer, checks the SHA-256 GitHub lists for the asset and runs it silently (`/S /R /UPDATE`:
//! the NSIS installer closes this process, replaces the files and starts the app again).
//!
//! - Only the newest release of this repository counts, and only an asset of that release's own
//!   download URL; the installer is not signed, so a hash that is missing or different stops the update.
//! - Only an installed copy updates itself (the exe sits in the profile folder, where the installer
//!   puts it): a build run from `target/` never replaces the installed app.
//! - Everything here blocks: call it from a worker thread, never from the UI thread.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

const LATEST: &str = "https://api.github.com/repos/bps2414/feathertune/releases/latest";
const DOWNLOADS: &str = "https://github.com/bps2414/feathertune/releases/download/";
const MAX_INSTALLER_BYTES: u64 = 64 * 1024 * 1024;

pub struct Update {
    pub version: String,
    name: String,
    url: String,
    sha256: String,
}

/// Whether this exe is the installed one (in `profile`, the install folder).
pub fn installed(profile: &Path) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let norm = |p: &Path| p.to_string_lossy().to_ascii_lowercase().replace('/', "\\");
    exe.parent().is_some_and(|dir| norm(dir).trim_end_matches('\\') == norm(profile).trim_end_matches('\\'))
}

fn agent(redirects: u32, timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .tls_config(
                TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build(),
            )
            .user_agent(concat!("feathertune/", env!("CARGO_PKG_VERSION")))
            .max_redirects(redirects)
            .timeout_global(Some(timeout))
            .build(),
    )
}

/// `Ok(None)`: this is the newest version.
pub fn check() -> Result<Option<Update>, String> {
    let mut response = agent(0, Duration::from_secs(6))
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    pick(&json, env!("CARGO_PKG_VERSION"))
}

/// The installer of the release in `json` if it is newer than `current`.
fn pick(json: &serde_json::Value, current: &str) -> Result<Option<Update>, String> {
    let tag = json["tag_name"].as_str().ok_or("no tag_name")?;
    let (latest, current) = (version(tag).ok_or("unreadable release tag")?, version(current).ok_or("bad own version")?);
    if latest <= current {
        return Ok(None);
    }
    let asset = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| n.starts_with("feathertune") && n.ends_with("-setup.exe")))
        .ok_or("the release has no installer")?;
    let name = asset["name"].as_str().ok_or("no asset name")?;
    let url = asset["browser_download_url"].as_str().filter(|u| u.starts_with(DOWNLOADS)).ok_or("unexpected URL")?;
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("the release lists no SHA-256 for the installer")?;
    // The name ends up in a path: keep it to what a file name of ours looks like.
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) {
        return Err("unexpected installer name".into());
    }
    Ok(Some(Update {
        version: tag.trim_start_matches('v').into(),
        name: name.into(),
        url: url.into(),
        sha256: sha256.to_ascii_lowercase(),
    }))
}

/// `1.2.3` or `v1.2.3` (a pre-release or build suffix is ignored).
pub fn version(text: &str) -> Option<(u64, u64, u64)> {
    let core = text.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let v = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(v)
}

/// Downloads the installer into `<profile>/update/`, reporting progress (0..1, or `None` when the
/// size is unknown), checks its hash and starts it. On success the installer is running and will
/// close this process: the caller exits. On error nothing was started.
pub fn install(update: &Update, profile: &Path, mut progress: impl FnMut(Option<f32>)) -> Result<(), String> {
    let dir = profile.join("update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&update.name);
    let result = download(update, &path, &mut progress).and_then(|()| {
        Command::new(&path)
            .args(["/S", "/R", "/UPDATE"])
            .spawn()
            .map(drop)
            .map_err(|e| format!("could not start it: {e}"))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

fn download(update: &Update, path: &PathBuf, progress: &mut impl FnMut(Option<f32>)) -> Result<(), String> {
    let response = agent(5, Duration::from_secs(180)).get(&update.url).call().map_err(|e| e.to_string())?;
    let total = response.body().content_length();
    let mut reader = response.into_body().into_with_config().limit(MAX_INSTALLER_BYTES).reader();
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let (mut hash, mut done, mut buf) = (Sha256::new(), 0u64, vec![0u8; 64 * 1024]);
    let mut last = -1.0f32;
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        let fraction = total.filter(|t| *t > 0).map(|t| (done as f32 / t as f32).min(1.0));
        // About one repaint per percent.
        if fraction.is_none_or(|f| f - last >= 0.01) {
            last = fraction.unwrap_or(last);
            progress(fraction);
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    let got: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != update.sha256 {
        return Err("the downloaded installer does not match the published SHA-256".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, digest: Option<&str>) -> serde_json::Value {
        json!({ "tag_name": tag, "assets": [
            { "name": "notes.txt", "browser_download_url": "https://github.com/bps2414/feathertune/releases/download/x/notes.txt" },
            { "name": "feathertune_0.2.0_x64-setup.exe",
              "browser_download_url": "https://github.com/bps2414/feathertune/releases/download/v0.2.0/feathertune_0.2.0_x64-setup.exe",
              "digest": digest }
        ]})
    }

    const HASH: &str = "sha256:35a8e7c815c9d3b94b878f6c978b71da493cde163ff6d30412a439c8fbc50629";

    #[test]
    fn versions_compare_numerically() {
        assert!(version("v0.10.0") > version("0.9.9"));
        assert_eq!(version("v1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(version("1.2"), None);
        assert_eq!(version("1.2.3.4"), None);
    }

    #[test]
    fn newer_release_is_offered_only_with_a_hash() {
        let u = pick(&release("v0.2.0", Some(HASH)), "0.1.2").unwrap().unwrap();
        assert_eq!(u.version, "0.2.0");
        assert_eq!(u.name, "feathertune_0.2.0_x64-setup.exe");
        assert!(pick(&release("v0.2.0", None), "0.1.2").is_err());
        assert!(pick(&release("v0.2.0", Some("sha256:abc")), "0.1.2").is_err());
    }

    #[test]
    fn same_or_older_release_is_not_an_update() {
        assert!(pick(&release("v0.1.2", Some(HASH)), "0.1.2").unwrap().is_none());
        assert!(pick(&release("v0.1.0", Some(HASH)), "0.1.2").unwrap().is_none());
    }

    #[test]
    fn foreign_download_url_is_refused() {
        let mut r = release("v0.2.0", Some(HASH));
        r["assets"][1]["browser_download_url"] = json!("https://example.com/feathertune-setup.exe");
        assert!(pick(&r, "0.1.2").is_err());
    }
}
