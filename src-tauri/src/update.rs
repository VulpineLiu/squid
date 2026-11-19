//! Git-backed update channel, matching the model used by nex_shift.
//!
//! Installers and a `squid-update.json` manifest live on an orphan `releases`
//! branch in this repository. A checkout over HTTPS is the whole update server.
//! The app verifies the installer's exact byte length and SHA256 before it can
//! run, and verifies a detached Ed25519 signature when a release key was baked
//! into the build. A helper replaces the app after it exits.

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use squid_core::settings::Settings;
use tauri::{AppHandle, Emitter, Manager};

const BAKED_MANIFEST_URL: &str = match option_env!("SQUID_UPDATE_URL") {
    Some(url) => url,
    None => "",
};
pub(crate) const SIGNATURE_FILE: &str = "squid-update.sig";
const ENV_MANIFEST_URL: &str = "SQUID_UPDATE_URL";
const BAKED_PUBLIC_KEY: &str = match option_env!("SQUID_UPDATE_PUBKEY") {
    Some(key) => key,
    None => "",
};
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const SUPPORTED_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Artifact {
    file: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReleaseManifest {
    schema: u32,
    version: String,
    date: String,
    notes: String,
    artifacts: BTreeMap<String, Artifact>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: String,
    pub current_version: String,
    pub available_version: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub notes: String,
    pub date: String,
    pub path: Option<String>,
    pub message: String,
    pub feed_url: String,
    pub signed: bool,
    pub artifact_key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProgress {
    downloaded: u64,
    total: u64,
    state: String,
}

#[derive(Default)]
pub struct UpdateState {
    status: std::sync::Mutex<Option<UpdateStatus>>,
    manifest: std::sync::Mutex<Option<ReleaseManifest>>,
    signed: std::sync::Mutex<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatePrefs {
    pub update_check_enabled: bool,
    last_update_check_day: i64,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self {
            update_check_enabled: true,
            last_update_check_day: 0,
        }
    }
}

fn prefs_path() -> PathBuf {
    Settings::config_dir().join("updates.json")
}

pub fn load_prefs() -> UpdatePrefs {
    fs::read_to_string(prefs_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_prefs(prefs: &UpdatePrefs) -> Result<(), String> {
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(prefs).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

fn unix_day() -> i64 {
    chrono::Utc::now().timestamp() / 86_400
}

fn update_check_due(interval: Duration) -> bool {
    let prefs = load_prefs();
    if !prefs.update_check_enabled {
        return false;
    }
    let elapsed = unix_day().saturating_sub(prefs.last_update_check_day);
    elapsed >= (interval.as_secs() / 86_400).max(1) as i64
}

fn note_update_check() {
    let mut prefs = load_prefs();
    prefs.last_update_check_day = unix_day();
    let _ = save_prefs(&prefs);
}

#[tauri::command]
pub async fn set_update_check_enabled(enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut prefs = load_prefs();
        prefs.update_check_enabled = enabled;
        save_prefs(&prefs)?;
        Ok(enabled)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn current_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn manifest_url() -> String {
    std::env::var(ENV_MANIFEST_URL)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| (!BAKED_MANIFEST_URL.is_empty()).then(|| BAKED_MANIFEST_URL.to_string()))
        .unwrap_or_default()
}

fn platform_key() -> &'static str {
    std::env::consts::OS
}

fn artifact_keys_for(platform: &str, arch: &str) -> Vec<String> {
    if platform == "windows" && arch == "aarch64" {
        return vec!["windows-arm64".to_string(), "windows".to_string()];
    }
    vec![platform.to_string()]
}

fn select_artifact<'a>(
    manifest: &'a ReleaseManifest,
    platform: &str,
    arch: &str,
) -> Option<(String, &'a Artifact)> {
    artifact_keys_for(platform, arch)
        .into_iter()
        .find_map(|key| manifest.artifacts.get(&key).map(|item| (key, item)))
}

fn artifact_for(manifest: &ReleaseManifest) -> Option<(String, &Artifact)> {
    select_artifact(manifest, platform_key(), std::env::consts::ARCH)
}

pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let (candidate_number, candidate_pre) = split_version(candidate);
    let (current_number, current_pre) = split_version(current);
    for index in 0..candidate_number.len().max(current_number.len()) {
        let left = candidate_number.get(index).copied().unwrap_or(0);
        let right = current_number.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    match (&candidate_pre, &current_pre) {
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (None, None) => false,
        (Some(left), Some(right)) => prerelease_is_newer(left, right),
    }
}

fn split_version(value: &str) -> (Vec<u64>, Option<Vec<&str>>) {
    let trimmed = value.trim().trim_start_matches('v');
    let core = trimmed.split('+').next().unwrap_or(trimmed);
    let (numbers, pre_release) = match core.split_once('-') {
        Some((head, tail)) => (head, (!tail.is_empty()).then(|| tail.split('.').collect())),
        None => (core, None),
    };
    (
        numbers
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect(),
        pre_release,
    )
}

fn prerelease_is_newer(candidate: &[&str], current: &[&str]) -> bool {
    for index in 0..candidate.len().max(current.len()) {
        match (candidate.get(index), current.get(index)) {
            (None, Some(_)) => return false,
            (Some(_), None) => return true,
            (None, None) => return false,
            (Some(left), Some(right)) => {
                if left == right {
                    continue;
                }
                return match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left > right,
                    (Ok(_), Err(_)) => false,
                    (Err(_), Ok(_)) => true,
                    (Err(_), Err(_)) => left > right,
                };
            }
        }
    }
    false
}

fn public_key() -> Result<Option<VerifyingKey>, String> {
    key_from_hex(BAKED_PUBLIC_KEY)
}

pub(crate) fn key_from_hex(value: &str) -> Result<Option<VerifyingKey>, String> {
    let key = value.trim();
    if key.is_empty() {
        return Ok(None);
    }
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(key, &mut bytes)
        .map_err(|_| "The configured release key is not 32 bytes of hex.".to_string())?;
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| "The configured release key is not an ed25519 public key.".to_string())
        .map(Some)
}

pub(crate) fn signature_url() -> Option<String> {
    let url = manifest_url();
    (!url.is_empty()).then(|| {
        let base = match url.rsplit_once('/') {
            Some((head, _)) => head,
            None => url.as_str(),
        };
        format!("{base}/{SIGNATURE_FILE}")
    })
}

/// The rule-pack channel lives beside the installer manifest but is a different
/// document with a different key. Rotating the release key must not silently
/// rotate who can push rules, and a leaked installer key must not become a
/// rule-injection path, so the two are never read from the same variable.
pub(crate) const INTEL_MANIFEST_FILE: &str = "squid-intel.json";
pub(crate) const INTEL_SIGNATURE_FILE: &str = "squid-intel.sig";
const BAKED_INTEL_PUBLIC_KEY: &str = match option_env!("SQUID_INTEL_PUBKEY") {
    Some(key) => key,
    None => "",
};

/// `…/squid-intel.json` on the same channel as the installers, or `None` when
/// this build has no feed at all.
pub(crate) fn intel_manifest_url() -> Option<String> {
    raw_asset_url(INTEL_MANIFEST_FILE).ok()
}

pub(crate) fn intel_manifest_signature() -> Result<Option<String>, String> {
    let url = match intel_manifest_url() {
        Some(_) => raw_asset_url(INTEL_SIGNATURE_FILE)?,
        None => return Ok(None),
    };
    // An absent signature file is an answer, not a failure: it means the channel
    // publishes unsigned rules, which `verify_against` then reports honestly.
    match fetch_text(&url) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.contains("404") || error.contains("not found") => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn intel_public_key() -> Result<Option<VerifyingKey>, String> {
    key_from_hex(BAKED_INTEL_PUBLIC_KEY)
}

fn verify_manifest(text: &str, signature: Option<&str>) -> Result<bool, String> {
    verify_against(public_key()?.as_ref(), text, signature)
}

pub(crate) fn verify_against(
    key: Option<&VerifyingKey>,
    text: &str,
    signature: Option<&str>,
) -> Result<bool, String> {
    let Some(key) = key else {
        return Ok(false);
    };
    let encoded = signature.ok_or_else(|| {
        "This feed has no squid-update.sig, but this build requires a signed manifest.".to_string()
    })?;
    let decoded = BASE64
        .decode(encoded.trim().as_bytes())
        .map_err(|error| format!("The feed's signature is not valid base64: {error}"))?;
    let signature = Signature::from_slice(&decoded)
        .map_err(|error| format!("The feed's signature is malformed: {error}"))?;
    key.verify_strict(text.as_bytes(), &signature)
        .map_err(|_| "The manifest signature does not match the release key.".to_string())?;
    Ok(true)
}

fn is_safe_artifact_name(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name)
            .file_name()
            .map(|part| part == name)
            .unwrap_or(false)
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || " ._+-".contains(character))
}

fn parse_manifest(text: &str) -> Result<ReleaseManifest, String> {
    let manifest: ReleaseManifest =
        serde_json::from_str(text).map_err(|error| format!("Malformed manifest: {error}"))?;
    if manifest.version.trim().is_empty() {
        return Err("The manifest carries no version.".into());
    }
    if manifest.schema > SUPPORTED_SCHEMA {
        return Err(format!(
            "Manifest schema {} is newer than this build understands.",
            manifest.schema
        ));
    }
    for artifact in manifest.artifacts.values() {
        if !is_sha256_hex(&artifact.sha256) {
            return Err(format!("{} has no usable SHA256.", artifact.file));
        }
        if !is_safe_artifact_name(&artifact.file) {
            return Err(format!(
                "{} is not a safe artifact file name.",
                artifact.file
            ));
        }
    }
    Ok(manifest)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn raw_asset_url(file: &str) -> Result<String, String> {
    let base = manifest_url();
    if base.is_empty() {
        return Err("No update feed is configured.".into());
    }
    let directory = base.rsplit_once('/').map(|(head, _)| head).unwrap_or(&base);
    Ok(format!("{directory}/{}", file.trim_start_matches('/')))
}

pub(crate) fn download(
    url: &str,
    destination: &Path,
    timeout: Duration,
    mut on_tick: impl FnMut(),
) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_file(destination);

    #[cfg(not(target_os = "windows"))]
    let (program, args): (&str, Vec<String>) = (
        "/usr/bin/curl",
        vec![
            "--fail".into(),
            "--silent".into(),
            "--show-error".into(),
            "--location".into(),
            "--max-time".into(),
            timeout.as_secs().to_string(),
            "--output".into(),
            destination.to_string_lossy().to_string(),
            url.to_string(),
        ],
    );
    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<String>) = {
        let escaped = destination.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ProgressPreference='SilentlyContinue'; Invoke-WebRequest -UseBasicParsing -TimeoutSec {} -Uri '{}' -OutFile '{}'",
            timeout.as_secs(),
            url.replace('\'', "''"),
            escaped
        );
        (
            "powershell.exe",
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                script,
            ],
        )
    };

    let mut command: Command = crate::process::command(program);
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start the download: {error}"))?;
    let deadline = Instant::now() + timeout + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("The download stalled for {timeout:?}."));
            }
            Ok(None) => {
                on_tick();
                thread::sleep(Duration::from_millis(250));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
    };
    if !status.success() {
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        let detail = detail.trim().to_string();
        return Err(if detail.is_empty() {
            format!(
                "The download failed (exit {}).",
                status.code().unwrap_or(-1)
            )
        } else {
            detail
        });
    }
    if destination.is_file() {
        Ok(())
    } else {
        Err("The download produced no file.".into())
    }
}

pub(crate) fn fetch_text(url: &str) -> Result<String, String> {
    let target = std::env::temp_dir().join(format!(
        "squid-manifest-{}-{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    download(url, &target, FETCH_TIMEOUT, || {})?;
    let text = fs::read_to_string(&target).map_err(|error| error.to_string())?;
    let _ = fs::remove_file(&target);
    Ok(text)
}

fn download_directory() -> PathBuf {
    std::env::temp_dir().join("Squid Updates")
}

pub fn sha256_of(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn emit_progress(app: &AppHandle, downloaded: u64, total: u64, state: &str) {
    let _ = app.emit(
        "update://progress",
        UpdateProgress {
            downloaded,
            total,
            state: state.to_string(),
        },
    );
}

fn publish(app: &AppHandle, status: UpdateStatus) -> UpdateStatus {
    if let Ok(mut guard) = app.state::<UpdateState>().status.lock() {
        *guard = Some(status.clone());
    }
    let _ = app.emit("update://status", status.clone());
    status
}

fn error_status(message: impl Into<String>) -> UpdateStatus {
    UpdateStatus {
        state: "error".into(),
        current_version: current_version(),
        available_version: String::new(),
        file_name: String::new(),
        size: 0,
        sha256: String::new(),
        notes: String::new(),
        date: String::new(),
        path: None,
        message: message.into(),
        feed_url: manifest_url(),
        signed: false,
        artifact_key: String::new(),
    }
}

fn status_for(manifest: &ReleaseManifest, path: Option<PathBuf>, signed: bool) -> UpdateStatus {
    let artifact = artifact_for(manifest);
    let (file_name, size, sha256, artifact_key) = match artifact.as_ref() {
        Some((key, item)) => (
            item.file.clone(),
            item.size,
            item.sha256.clone(),
            key.clone(),
        ),
        None => (String::new(), 0, String::new(), String::new()),
    };
    let newer = version_is_newer(&manifest.version, &current_version());
    let message = if newer && artifact.is_none() {
        format!(
            "Release {} has no artifact for this platform ({}).",
            manifest.version,
            artifact_keys_for(platform_key(), std::env::consts::ARCH).join(" or ")
        )
    } else {
        String::new()
    };
    UpdateStatus {
        state: match (newer, artifact) {
            (true, Some(_)) => "available",
            (true, None) => "unavailable",
            (false, _) => "current",
        }
        .to_string(),
        current_version: current_version(),
        available_version: manifest.version.clone(),
        file_name,
        size,
        sha256,
        notes: manifest.notes.clone(),
        date: manifest.date.clone(),
        path: path.map(|value| value.to_string_lossy().to_string()),
        message,
        feed_url: manifest_url(),
        signed,
        artifact_key,
    }
}

fn remembered_signed(app: &AppHandle) -> bool {
    app.state::<UpdateState>()
        .signed
        .lock()
        .map(|guard| *guard)
        .unwrap_or(false)
}

fn remembered_manifest(app: &AppHandle) -> Option<ReleaseManifest> {
    app.state::<UpdateState>()
        .manifest
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckOutcome {
    pub status: UpdateStatus,
    pub download_ready: bool,
}

#[tauri::command]
pub async fn update_check(app: AppHandle) -> UpdateCheckOutcome {
    let outcome = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || {
            let url = manifest_url();
            if url.is_empty() {
                return Err("This build has no update feed configured.".to_string());
            }
            let text = fetch_text(&url)?;
            let signature = signature_url().and_then(|candidate| fetch_text(&candidate).ok());
            let signed = verify_manifest(&text, signature.as_deref())?;
            let manifest = parse_manifest(&text)?;
            if let Ok(mut guard) = app.state::<UpdateState>().manifest.lock() {
                *guard = Some(manifest.clone());
            }
            if let Ok(mut guard) = app.state::<UpdateState>().signed.lock() {
                *guard = signed;
            }
            Ok::<ReleaseManifest, String>(manifest)
        }
    })
    .await;

    match outcome.unwrap_or_else(|error| Err(error.to_string())) {
        Ok(manifest) => {
            let status = status_for(&manifest, None, remembered_signed(&app));
            let download_ready = status.state == "available";
            UpdateCheckOutcome {
                download_ready,
                status: publish(&app, status),
            }
        }
        Err(error) => UpdateCheckOutcome {
            download_ready: false,
            status: publish(&app, error_status(error)),
        },
    }
}

#[tauri::command]
pub fn update_status(state: tauri::State<'_, UpdateState>) -> Option<UpdateStatus> {
    state.status.lock().ok().and_then(|guard| guard.clone())
}

#[tauri::command]
pub async fn update_download(app: AppHandle) -> Result<UpdateStatus, String> {
    let manifest =
        remembered_manifest(&app).ok_or_else(|| "Run a check for updates first.".to_string())?;
    let artifact = artifact_for(&manifest)
        .map(|(_, item)| item.clone())
        .ok_or_else(|| {
            format!(
                "Release {} has no artifact for this platform.",
                manifest.version
            )
        })?;
    let target = download_directory().join(&artifact.file);
    let reporter = app.clone();
    let expected = artifact.sha256.clone();
    let total = artifact.size;
    tauri::async_runtime::spawn_blocking(move || {
        emit_progress(&reporter, 0, total, "downloading");
        let written = std::sync::atomic::AtomicU64::new(0);
        download(
            &raw_asset_url(&artifact.file)?,
            &target,
            DOWNLOAD_TIMEOUT,
            || {
                let size = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
                if size > written.load(std::sync::atomic::Ordering::Relaxed) {
                    written.store(size, std::sync::atomic::Ordering::Relaxed);
                    emit_progress(&reporter, size, total, "downloading");
                }
            },
        )?;
        let actual_size = fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0);
        if total > 0 && actual_size != total {
            let _ = fs::remove_file(&target);
            return Err(format!(
                "Expected {} bytes but received {actual_size}. The download was discarded.",
                total
            ));
        }
        emit_progress(&reporter, actual_size, total, "verifying");
        let actual = sha256_of(&target)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            let _ = fs::remove_file(&target);
            return Err(format!(
                "SHA256 mismatch: expected {expected}, got {actual}. The download was discarded."
            ));
        }
        let mut status = status_for(
            &manifest,
            Some(target.clone()),
            remembered_signed(&reporter),
        );
        status.state = "ready".into();
        status.message = actual;
        Ok(publish(&reporter, status))
    })
    .await
    .map_err(|error| format!("Update download failed: {error}"))?
}

#[cfg(target_os = "macos")]
fn bundle_path(exe: &Path) -> Option<PathBuf> {
    let contents = exe.parent()?.parent()?;
    let bundle = contents.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    bundle
        .extension()
        .filter(|ext| ext.eq_ignore_ascii_case("app"))?;
    Some(bundle.to_path_buf())
}

#[cfg(target_os = "macos")]
fn can_replace(bundle: &Path) -> bool {
    let parent = match bundle.parent() {
        Some(parent) => parent,
        None => return false,
    };
    let probe = parent.join(format!(".squid-replace-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
fn macos_handoff(pid: u32, image: &Path, bundle: &Path, log: &Path) -> String {
    format!(
        r#"#!/bin/sh
pid="{pid}"
image="{image}"
bundle="{bundle}"
log="{log}"
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
mount="$TMPDIR/Squid Updates/mount.$$"
[ -n "$TMPDIR" ] || mount="/tmp/Squid Updates/mount.$$"
mkdir -p "${{mount%/*}}"
if ! /usr/bin/hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$image" >>"$log" 2>&1; then
  echo "mount failed" >>"$log"
  /usr/bin/open "$bundle"
  exit 1
fi
status=0
if ! /usr/bin/ditto "$mount/Squid.app" "$bundle.new" >>"$log" 2>&1; then
  status=1
else
  /usr/bin/xattr -dr com.apple.quarantine "$bundle.new" 2>/dev/null
  if mv "$bundle" "$bundle.old" >>"$log" 2>&1 && mv "$bundle.new" "$bundle" >>"$log" 2>&1; then
    /bin/rm -rf "$bundle.old"
  else
    status=1
    [ -d "$bundle.old" ] && [ ! -d "$bundle" ] && mv "$bundle.old" "$bundle" >>"$log" 2>&1
    /bin/rm -rf "$bundle.new"
  fi
fi
/usr/bin/hdiutil detach "$mount" -quiet >>"$log" 2>&1
[ "$status" -eq 0 ] || echo "handoff failed; reopening the previous copy" >>"$log"
/usr/bin/open "$bundle"
exit "$status"
"#,
        pid = pid,
        image = image.display(),
        bundle = bundle.display(),
        log = log.display(),
    )
}

#[cfg(target_os = "windows")]
fn windows_handoff(pid: u32, installer: &Path, log: &Path) -> String {
    format!(
        r#"@echo off
setlocal EnableExtensions
set "WAIT_FOR={pid}"
set "INSTALLER={installer}"
set "LOG={log}"
set /a TRIES=0
:waitloop
tasklist /FI "PID eq %WAIT_FOR%" /NH /FO CSV 2>nul | find /I "squid-desktop.exe" >nul || goto install
set /a TRIES+=1
if %TRIES% LSS 100 (ping -n 2 127.0.0.1 >nul & goto waitloop)
:install
set "TARGET=%LOCALAPPDATA%\Programs\Squid\squid-desktop.exe"
for /f "tokens=1,2,*" %%a in ('reg query "HKCU\Software\Squid" /v InstallDirectory 2^>nul ^| find "InstallDirectory"') do set "TARGET=%%c\squid-desktop.exe"
start /wait "" "%INSTALLER%" /S
if errorlevel 1 echo installer exited with %ERRORLEVEL% >> "%LOG%"
if exist "%TARGET%" start "" "%TARGET%"
"#,
        pid = pid,
        installer = installer.display(),
        log = log.display(),
    )
}

fn spawn_handoff(name: &str, script: &str) -> Result<(), String> {
    let path = download_directory().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_file(&path);
    fs::write(&path, script).map_err(|error| error.to_string())?;
    #[cfg(target_os = "windows")]
    let mut command = crate::process::command("cmd.exe");
    #[cfg(target_os = "windows")]
    let script = path.to_string_lossy();
    #[cfg(target_os = "windows")]
    command.args(["/D", "/C", &*script]);
    #[cfg(not(target_os = "windows"))]
    let mut command = crate::process::command("/bin/sh");
    #[cfg(not(target_os = "windows"))]
    command.arg(&path);
    command
        .spawn()
        .map_err(|error| format!("Could not start the updater: {error}"))?;
    Ok(())
}

fn quit_for_update(app: &AppHandle) {
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(600));
        app.exit(0);
    });
}

#[cfg(not(target_os = "windows"))]
fn open_artifact(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::process::command("/usr/bin/open")
            .arg(path)
            .status()
            .map_err(|error| format!("Could not open the update: {error}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        crate::process::command("cmd.exe")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .status()
            .map_err(|error| format!("Could not open the update: {error}"))?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        crate::process::command("xdg-open")
            .arg(path)
            .status()
            .map_err(|error| format!("Could not open the update: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<String, String> {
    let path = app
        .state::<UpdateState>()
        .status
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().and_then(|status| status.path.clone()))
        .ok_or_else(|| "Download and verify an update first.".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let artifact = PathBuf::from(&path);
        let log = download_directory().join("handoff.log");
        let pid = std::process::id();

        #[cfg(target_os = "macos")]
        {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            if let Some(bundle) = bundle_path(&exe) {
                if !can_replace(&bundle) {
                    let folder = bundle
                        .parent()
                        .map_or_else(|| "its folder".to_string(), |parent| {
                            parent.to_string_lossy().to_string()
                        });
                    return Err(format!(
                        "Squid cannot replace itself in {folder}. Open the downloaded installer to update by hand."
                    ));
                }
                let script = macos_handoff(pid, &artifact, &bundle, &log);
                spawn_handoff("Squid-update.sh", &script)?;
                quit_for_update(&app);
                return Ok(path);
            }
        }
        #[cfg(target_os = "windows")]
        {
            let script = windows_handoff(pid, &artifact, &log);
            spawn_handoff("Squid-update.cmd", &script)?;
            quit_for_update(&app);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            open_artifact(&artifact)?;
        }
        #[cfg(target_os = "macos")]
        open_artifact(&artifact)?;
        Ok(path)
    })
    .await
    .map_err(|error| format!("Update install failed: {error}"))?
}

#[tauri::command]
pub fn update_clear(app: AppHandle) {
    if let Ok(mut guard) = app.state::<UpdateState>().status.lock() {
        if let Some(path) = guard.as_ref().and_then(|status| status.path.as_ref()) {
            let _ = fs::remove_file(path);
        }
        *guard = None;
    }
    let _ = app.emit("update://status", serde_json::Value::Null);
}

pub fn spawn_startup_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if manifest_url().is_empty() || !update_check_due(CHECK_INTERVAL) {
            return;
        }
        note_update_check();
        let _ = update_check(app).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn feed_guard() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: StdMutex<()> = StdMutex::new(());
        GUARD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn semver_releases_outrank_prereleases() {
        assert!(version_is_newer("0.2.0", "0.1.9"));
        assert!(version_is_newer("0.2.0", "0.2.0-beta.2"));
        assert!(!version_is_newer("0.2.0-beta.2", "0.2.0"));
        assert!(version_is_newer("0.2.0-beta.10", "0.2.0-beta.2"));
        assert!(!version_is_newer("0.2.0+build.9", "0.2.0"));
    }

    #[test]
    fn the_platform_ladder_never_crosses_operating_systems() {
        assert_eq!(
            artifact_keys_for("windows", "aarch64"),
            vec!["windows-arm64", "windows"]
        );
        assert_eq!(artifact_keys_for("macos", "aarch64"), vec!["macos"]);
        assert_eq!(artifact_keys_for("windows", "x86_64"), vec!["windows"]);
    }

    fn manifest() -> ReleaseManifest {
        ReleaseManifest {
            schema: 1,
            version: "0.2.0".into(),
            date: "2026-11-03".into(),
            notes: String::new(),
            artifacts: BTreeMap::from([
                (
                    "windows".into(),
                    Artifact {
                        file: "Squid-Setup-0.2.0-win-x64.exe".into(),
                        sha256: "a".repeat(64),
                        size: 12,
                    },
                ),
                (
                    "windows-arm64".into(),
                    Artifact {
                        file: "Squid-Setup-0.2.0-win-arm64.exe".into(),
                        sha256: "b".repeat(64),
                        size: 13,
                    },
                ),
            ]),
        }
    }

    #[test]
    fn arm_windows_prefers_its_native_artifact_then_falls_back() {
        let mut manifest = manifest();
        let (key, artifact) = select_artifact(&manifest, "windows", "aarch64").unwrap();
        assert_eq!(key, "windows-arm64");
        assert_eq!(artifact.size, 13);
        manifest.artifacts.remove("windows-arm64");
        let (key, artifact) = select_artifact(&manifest, "windows", "aarch64").unwrap();
        assert_eq!(key, "windows");
        assert_eq!(artifact.size, 12);
    }

    #[test]
    fn a_manifest_artifact_cannot_escape_the_update_folder() {
        let mut value = serde_json::json!({
            "schema": 1,
            "version": "0.2.0",
            "date": "2026-11-03",
            "notes": "",
            "artifacts": {
                "macos": {
                    "file": "../../evil.dmg",
                    "sha256": "a".repeat(64),
                    "size": 1
                }
            }
        });
        assert!(parse_manifest(&serde_json::to_string(&value).unwrap()).is_err());
        value["artifacts"]["macos"]["file"] = serde_json::json!("Squid-0.2.0-macOS-universal.dmg");
        assert!(parse_manifest(&serde_json::to_string(&value).unwrap()).is_ok());
    }

    #[test]
    fn a_newer_manifest_schema_is_refused() {
        let text = serde_json::to_string(&serde_json::json!({
            "schema": 2,
            "version": "9.0.0",
            "artifacts": {}
        }))
        .unwrap();
        assert!(parse_manifest(&text).is_err());
    }

    #[test]
    fn the_runtime_override_wins_over_the_baked_feed() {
        let _guard = feed_guard();
        std::env::set_var(ENV_MANIFEST_URL, "https://example.test/squid-update.json");
        assert_eq!(manifest_url(), "https://example.test/squid-update.json");
        std::env::remove_var(ENV_MANIFEST_URL);
    }
}
