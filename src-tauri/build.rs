//! Build-time wiring for the git-backed update channel.
//!
//! The feed lives in this repository's `releases` branch. The manifest URL is
//! derived from the `origin` remote, matching the mechanism used by nex_shift.
//! `SQUID_UPDATE_URL` and `SQUID_UPDATE_PUBKEY` are the two explicit overrides.

use std::{path::Path, process::Command};

const RELEASE_BRANCH: &str = "releases";
const MANIFEST: &str = "squid-update.json";
const PUBKEY_ENV: &str = "SQUID_UPDATE_PUBKEY";

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent()?)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn parse_remote(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some(rest) = url.strip_prefix("ssh://") {
        rest.split_once('/')?
    } else if let Some(rest) = url.find("://").map(|index| &url[index + 3..]) {
        rest.split_once('/')?
    } else if let Some((host, path)) = url.split_once(':') {
        (host.strip_prefix("git@")?, path)
    } else {
        return None;
    };
    (!host.is_empty() && !path.is_empty()).then(|| (format!("https://{host}"), path.to_string()))
}

fn raw_directory(host: &str, path: &str) -> String {
    let domain = host.replace("https://", "");
    let segment = if domain.ends_with("github.com") {
        format!("raw/{RELEASE_BRANCH}")
    } else if domain.contains("gitlab") {
        format!("-/raw/{RELEASE_BRANCH}")
    } else {
        format!("raw/branch/{RELEASE_BRANCH}")
    };
    format!("{host}/{path}/{segment}")
}

fn manifest_url() -> Option<String> {
    if let Ok(url) = std::env::var("SQUID_UPDATE_URL") {
        if !url.trim().is_empty() {
            return Some(url.trim().to_string());
        }
    }
    let (host, path) = parse_remote(&git(&["remote", "get-url", "origin"])?)?;
    Some(format!("{}/{}", raw_directory(&host, &path), MANIFEST))
}

fn public_key() -> Result<String, String> {
    let raw = match std::env::var(PUBKEY_ENV) {
        Ok(value) => value,
        Err(_) => return Ok(String::new()),
    };
    let value: String = raw
        .trim()
        .chars()
        .filter(|character| !character.is_whitespace() && *character != ':')
        .collect::<String>()
        .to_ascii_lowercase();
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{PUBKEY_ENV} must be 64 hex characters (32 bytes)"));
    }
    Ok(value)
}

fn main() {
    println!("cargo:rerun-if-changed=../dist");
    println!("cargo:rerun-if-env-changed=SQUID_UPDATE_URL");
    println!("cargo:rerun-if-env-changed={PUBKEY_ENV}");
    println!("cargo:rerun-if-changed=../.git/config");
    println!(
        "cargo:rustc-env=SQUID_UPDATE_URL={}",
        manifest_url().unwrap_or_default()
    );
    match public_key() {
        Ok(key) => println!("cargo:rustc-env=SQUID_UPDATE_PUBKEY={key}"),
        Err(error) => panic!("{error}"),
    }
    tauri_build::build()
}
