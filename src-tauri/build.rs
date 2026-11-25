//! Build-time wiring for the git-backed update channel.
//!
//! The feed lives in the origin repository's orphan `releases` branch. The
//! GitHub repository is a source mirror and deliberately does not carry that
//! branch, so the default URL is fixed instead of being derived from whatever
//! remote a checkout happens to use. `SQUID_UPDATE_URL` and
//! `SQUID_UPDATE_PUBKEY` are the two explicit overrides.

use std::env;

const DEFAULT_UPDATE_URL: &str =
    "https://git.unissense.tech/mula/nex_squid/raw/branch/releases/squid-update.json";
const PUBKEY_ENV: &str = "SQUID_UPDATE_PUBKEY";

fn manifest_url() -> String {
    if let Ok(url) = env::var("SQUID_UPDATE_URL") {
        if !url.trim().is_empty() {
            return url.trim().to_string();
        }
    }
    DEFAULT_UPDATE_URL.to_string()
}

fn public_key() -> Result<String, String> {
    let raw = match env::var(PUBKEY_ENV) {
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
    println!("cargo:rustc-env=SQUID_UPDATE_URL={}", manifest_url());
    match public_key() {
        Ok(key) => println!("cargo:rustc-env=SQUID_UPDATE_PUBKEY={key}"),
        Err(error) => panic!("{error}"),
    }
    tauri_build::build()
}
