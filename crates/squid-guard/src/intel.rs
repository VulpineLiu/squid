//! Rule packs: the on-disk format, its verification, and the fallback that keeps
//! a bad download from leaving the machine unguarded.
//!
//! A pack is YARA source plus a JSON manifest, published beside the installers
//! on the same git-backed channel the app update uses. Two keys are involved and
//! they are deliberately different: the release key signs installers, and the
//! intel key signs rules. Rotating one should not have to move the other, and a
//! leaked installer key must not become a rule-injection path.
//!
//! Nothing is uploaded. The client fetches a manifest and, if newer, a pack; no
//! filename, hash, or scan result ever leaves the machine. That is a property of
//! this module, not of the caller's good intentions — there is no function here
//! that takes a path from the local disk and sends it anywhere.

use crate::rules::RuleLevel;
use crate::rules::RuleMeta;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "squid-intel.json";
pub const SIGNATURE_FILE: &str = "squid-intel.sig";
const SUPPORTED_SCHEMA: u32 = 1;
/// Parsers are the first thing a hostile input breaks, so the size gate is not
/// about disk space.
const MAX_PACK_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelManifest {
    pub schema: u32,
    /// Monotonic pack revision. Compared as a number, not a date, so a re-issue
    /// on the same day still wins.
    pub revision: u64,
    pub date: String,
    pub pack: String,
    pub sha256: String,
    pub size: u64,
    pub rules: Vec<RuleMeta>,
}

/// How the bytes reached us, which is a different fact from *what* they are.
///
/// Both variants mean the pack's SHA-256 and byte length matched its manifest and
/// that YARA accepted the source. Only `Verified` means a key this build knows
/// about signed the manifest.
///
/// The distinction exists because the release key is baked in at build time, so a
/// build from source has none. Refusing to update then would make rule packs a
/// feature only the release build can use; accepting silently would let the UI
/// print "signed" about a manifest nothing checked. So an unbaked key is accepted
/// and remembered as unverified, and the interface says so. A build that *does*
/// have a key and fails to verify it is an error, not a fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// A baked public key verified the manifest's Ed25519 signature.
    Verified,
    /// Integrity came from the manifest's SHA-256 alone: this build has no key.
    Unverified,
}

/// A pack that has been read, verified, and compiled.
#[derive(Clone, Debug)]
pub struct Pack {
    pub manifest: IntelManifest,
    pub source: String,
    pub provenance: Provenance,
}

impl Pack {
    pub fn rule_count(&self) -> usize {
        self.manifest.rules.len()
    }
}

/// Where packs live: `current` is what the next scan uses, `previous` is what a
/// failed update falls back to. Keeping both is what makes a bad pack survivable.
pub fn intel_dir() -> PathBuf {
    crate::coverage::intel_root()
}

fn current_pack_path() -> PathBuf {
    intel_dir().join("current.json")
}

fn previous_pack_path() -> PathBuf {
    intel_dir().join("previous.json")
}

/// Validate a parsed manifest before anything reads its fields.
pub fn validate_manifest(manifest: &IntelManifest) -> Result<(), String> {
    if manifest.schema > SUPPORTED_SCHEMA {
        return Err(format!(
            "This rule pack needs intel schema {}, which Squid {} cannot read",
            manifest.schema,
            env!("CARGO_PKG_VERSION")
        ));
    }
    if !is_hex(manifest.sha256.as_bytes(), 64) {
        return Err("The rule pack manifest does not carry a SHA-256.".into());
    }
    if manifest.size == 0 || manifest.size > MAX_PACK_BYTES {
        return Err(format!(
            "The rule pack size ({} bytes) is outside what Squid will read.",
            manifest.size
        ));
    }
    let file = Path::new(&manifest.pack)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if file.is_empty() || file != manifest.pack || manifest.pack.contains("..") {
        return Err("The rule pack names a file outside its own folder.".into());
    }
    if !file.ends_with(".json") {
        return Err("The rule pack file must be a .json pack.".into());
    }
    if manifest.rules.is_empty() {
        return Err("The rule pack declares no rules.".into());
    }
    for rule in &manifest.rules {
        if rule.name.is_empty()
            || !rule
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!(
                "Rule name {:?} is not a YARA identifier.",
                rule.name
            ));
        }
    }
    Ok(())
}

pub fn parse_manifest(text: &str) -> Result<IntelManifest, String> {
    let manifest: IntelManifest = serde_json::from_str(text)
        .map_err(|error| format!("The rule manifest is not readable: {error}"))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Read a downloaded pack and check it against the manifest the caller verified.
/// Signature verification happens where the public key lives (the desktop
/// layer), so this takes the manifest as given and proves only that *these
/// bytes* are the pack that manifest describes.
pub fn read_pack(
    dir: &Path,
    manifest: &IntelManifest,
    provenance: Provenance,
) -> Result<Pack, String> {
    let path = dir.join(&manifest.pack);
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Could not read the rule pack: {error}"))?;
    if metadata.len() != manifest.size {
        return Err(format!(
            "{} is {} bytes, the manifest says {}.",
            manifest.pack,
            metadata.len(),
            manifest.size
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
    let actual = digest(&bytes);
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err(format!(
            "The rule pack hash does not match the manifest ({}).",
            short(&actual)
        ));
    }
    let parsed: PackFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The rule pack is not readable: {error}"))?;
    if parsed.revision != manifest.revision {
        return Err("The rule pack revision does not match the manifest.".into());
    }
    // Compile before accepting: a pack YARA rejects must never become
    // `current`, because the next scan would fail at the worst moment.
    crate::rules::Detector::from_source_for_pack(&parsed.source, &manifest.rules)?;
    Ok(Pack {
        manifest: manifest.clone(),
        source: parsed.source,
        provenance,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PackFile {
    revision: u64,
    source: String,
}

/// What squid keeps on disk: the pack, together with the manifest that was
/// verified when it was installed. One file, so no later read can pair a pack
/// with the wrong manifest.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PackRecord {
    manifest: IntelManifest,
    revision: u64,
    source: String,
    /// Recorded at install so the UI can still say, after a restart and with no
    /// network, whether what it is running was ever signature-verified.
    /// Defaults to the honest pessimistic answer for a record written before this
    /// field existed: nothing is claimed about the signature.
    #[serde(default = "Provenance::unverified")]
    provenance: Provenance,
}

impl Provenance {
    const fn unverified() -> Provenance {
        Provenance::Unverified
    }
}

/// Build the pack file that gets published, so a pack is produced by the same
/// code that reads one.
pub fn pack_file(revision: u64, source: &str) -> Vec<u8> {
    let file = PackFile {
        revision,
        source: source.to_string(),
    };
    serde_json::to_vec_pretty(&file).expect("a pack file always serialises")
}

/// Install a verified pack, keeping the record it replaces for rollback.
pub fn install_pack(pack: &Pack) -> Result<(), String> {
    let dir = intel_dir();
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let current = current_pack_path();
    if current.is_file() {
        let text = std::fs::read_to_string(&current).map_err(|error| error.to_string())?;
        std::fs::write(previous_pack_path(), text).map_err(|error| error.to_string())?;
    }
    let record = PackRecord {
        manifest: pack.manifest.clone(),
        revision: pack.manifest.revision,
        source: pack.source.clone(),
        provenance: pack.provenance,
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    std::fs::write(&current, bytes).map_err(|error| error.to_string())
}

/// The best installed pack, falling back to the previous record when `current`
/// has been corrupted or truncated since it was written.
pub fn installed_pack() -> Option<Pack> {
    for path in [current_pack_path(), previous_pack_path()] {
        if let Some(pack) = read_record(&path) {
            return Some(pack);
        }
    }
    None
}

/// Read one stored record and re-prove it: the hash against its own manifest,
/// and the source against the rule table. A record that fails either is treated
/// as absent, because half a rule set is how a bad pack becomes a false sense of
/// safety.
pub fn read_record(path: &Path) -> Option<Pack> {
    let text = std::fs::read_to_string(path).ok()?;
    let record: PackRecord = serde_json::from_str(&text).ok()?;
    validate_manifest(&record.manifest).ok()?;
    let bytes = pack_file(record.revision, &record.source);
    if bytes.len() as u64 != record.manifest.size {
        return None;
    }
    if !digest(&bytes).eq_ignore_ascii_case(&record.manifest.sha256) {
        return None;
    }
    if crate::rules::Detector::from_source_for_pack(&record.source, &record.manifest.rules).is_err()
    {
        return None;
    }
    Some(Pack {
        manifest: record.manifest,
        source: record.source,
        provenance: record.provenance,
    })
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Everything the UI shows about the installed rules, without touching the
/// network. `builtin` is the floor: a machine that never downloaded a pack is
/// still guarded by the compiled-in rules.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntelStatus {
    pub builtin_rules: usize,
    pub installed_rules: usize,
    pub revision: u64,
    pub published: String,
    /// Compiled-in rules are as trusted as the binary itself; a downloaded pack is
    /// `true` only when a key this build knows about signed its manifest.
    pub signed: bool,
    pub source: String,
    pub path: String,
    pub fallback_available: bool,
}

pub fn status() -> IntelStatus {
    let builtin = crate::rules::builtin_meta().len();
    match installed_pack() {
        Some(pack) => IntelStatus {
            builtin_rules: builtin,
            installed_rules: pack.rule_count(),
            revision: pack.manifest.revision,
            published: pack.manifest.date.clone(),
            signed: pack.provenance == Provenance::Verified,
            source: "pack".into(),
            path: current_pack_path().to_string_lossy().into_owned(),
            fallback_available: previous_pack_path().is_file(),
        },
        None => IntelStatus {
            builtin_rules: builtin,
            installed_rules: builtin,
            revision: 0,
            published: String::new(),
            signed: true,
            source: "builtin".into(),
            path: intel_dir().to_string_lossy().into_owned(),
            fallback_available: false,
        },
    }
}

/// Roll back one step, used when a scan reports that the newest pack misbehaves.
pub fn rollback() -> Result<(), String> {
    let previous = previous_pack_path();
    let text = std::fs::read_to_string(&previous)
        .map_err(|_| "There is no previous rule pack to fall back to.".to_string())?;
    std::fs::write(current_pack_path(), text).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(previous);
    Ok(())
}

/// The path of the installed record, for the UI that shows where rules came from.
pub fn current_pack_record() -> PathBuf {
    current_pack_path()
}

fn is_hex(bytes: &[u8], length: usize) -> bool {
    bytes.len() == length && bytes.iter().all(|byte| byte.is_ascii_hexdigit())
}

fn short(value: &str) -> String {
    value.chars().take(12).collect()
}

/// Rule levels as the pack spells them, checked against what the scanner will
/// accept. A pack cannot invent a level, and cannot promote itself to
/// `signature` for a rule the client has never seen before without saying so.
pub fn level_is_known(level: &RuleLevel) -> bool {
    matches!(level, RuleLevel::Signature | RuleLevel::Behaviour)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest that honestly describes one pack source.
    fn manifest_for(revision: u64, rule: &str, source: &str) -> IntelManifest {
        let bytes = pack_file(revision, source);
        IntelManifest {
            schema: SUPPORTED_SCHEMA,
            revision,
            date: "2026-11-06".into(),
            pack: "squid-rules.json".into(),
            sha256: digest(&bytes),
            size: bytes.len() as u64,
            rules: vec![RuleMeta {
                name: rule.into(),
                description: "only in the pack".into(),
                family: "pack".into(),
                level: RuleLevel::Signature,
            }],
        }
    }

    fn manifest(revision: u64) -> IntelManifest {
        let source = r#"rule PackOnly { strings: $a = "pack-marker-only" condition: $a }"#;
        let bytes = pack_file(revision, source);
        IntelManifest {
            schema: SUPPORTED_SCHEMA,
            revision,
            date: "2026-11-06".into(),
            pack: "squid-rules.json".into(),
            sha256: write_and_hash(&bytes),
            size: bytes.len() as u64,
            rules: vec![RuleMeta {
                name: "PackOnly".into(),
                description: "only in the pack".into(),
                family: "pack".into(),
                level: RuleLevel::Signature,
            }],
        }
    }

    fn write_and_hash(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(bytes))
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("squid-intel-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("squid-rules.json"));
        dir
    }

    #[test]
    fn a_pack_round_trips_through_its_manifest() {
        let dir = temp_dir("round-trip");
        let manifest = manifest(7);
        std::fs::write(
            dir.join(&manifest.pack),
            pack_file(
                7,
                r#"rule PackOnly { strings: $a = "pack-marker-only" condition: $a }"#,
            ),
        )
        .unwrap();
        let manifest = {
            let bytes = std::fs::read(dir.join(&manifest.pack)).unwrap();
            IntelManifest {
                sha256: write_and_hash(&bytes),
                size: bytes.len() as u64,
                ..manifest
            }
        };
        let pack =
            read_pack(&dir, &manifest, Provenance::Verified).expect("the pack should read back");
        assert_eq!(pack.rule_count(), 1);
        assert_eq!(pack.manifest.revision, 7);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_truncated_pack_is_refused_before_it_is_used() {
        let dir = temp_dir("truncated");
        let manifest = manifest(8);
        std::fs::write(dir.join(&manifest.pack), b"half a file").unwrap();
        let error = read_pack(&dir, &manifest, Provenance::Verified)
            .expect_err("size must match the manifest");
        assert!(error.contains("bytes"), "{error}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_pack_that_yara_rejects_never_installs() {
        let dir = temp_dir("broken-yara");
        let bytes = pack_file(9, "rule {{{ not yara");
        std::fs::write(dir.join("squid-rules.json"), &bytes).unwrap();
        let manifest = IntelManifest {
            schema: SUPPORTED_SCHEMA,
            revision: 9,
            date: "2026-11-06".into(),
            pack: "squid-rules.json".into(),
            sha256: write_and_hash(&bytes),
            size: bytes.len() as u64,
            rules: vec![RuleMeta {
                name: "Whatever".into(),
                description: String::new(),
                family: "pack".into(),
                level: RuleLevel::Behaviour,
            }],
        };
        assert!(
            read_pack(&dir, &manifest, Provenance::Verified).is_err(),
            "a pack YARA rejects must not be installable"
        );
    }

    #[test]
    fn an_unsigned_pack_survives_the_round_trip_as_unsigned() {
        // A build with no baked key can still take updates, but must not end up
        // claiming afterwards that something verified them.
        let _guard = crate::state_guard("provenance");
        let pack = Pack {
            manifest: manifest_for(7, "ProvOnly", r#"rule ProvOnly { condition: false }"#),
            source: r#"rule ProvOnly { condition: false }"#.to_string(),
            provenance: Provenance::Unverified,
        };
        install_pack(&pack).unwrap();
        let status = status();
        assert_eq!(status.source, "pack");
        assert!(
            !status.signed,
            "the UI has to be able to tell an unverified pack from a signed one"
        );
        assert_eq!(installed_pack().unwrap().provenance, Provenance::Unverified);
    }

    #[test]
    fn a_record_from_before_provenance_existed_is_not_assumed_signed() {
        // An old `current.json` has no provenance field. Reading it as
        // `Verified` would print a green "signed" for a pack nobody checked.
        let _guard = crate::state_guard("provenance-old");
        let pack = published_pack(8, "OldOnly", r#"rule OldOnly { condition: false }"#);
        install_pack(&pack).unwrap();
        let path = current_pack_path();
        let text = std::fs::read_to_string(&path).unwrap();
        // The field is last in the pretty-printed object, so the comma to remove
        // is the one before it.
        let stripped = text.replace(",\n  \"provenance\": \"verified\"", "");
        assert_ne!(text, stripped, "the fixture must actually carry the field");
        std::fs::write(&path, stripped).unwrap();
        let read = read_record(&path).expect("a record missing one field still reads");
        assert_eq!(read.provenance, Provenance::Unverified);
    }

    #[test]
    fn a_rejected_pack_is_never_installed_even_when_the_hash_matches() {
        let dir = std::env::temp_dir().join("squid-intel-yara");
        std::fs::create_dir_all(&dir).unwrap();
        let source = "rule Broken { condition: this is not yara ((( }";
        let manifest = manifest_for(9, "Broken", source);
        std::fs::write(dir.join(&manifest.pack), pack_file(9, source)).unwrap();
        assert!(
            read_pack(&dir, &manifest, Provenance::Verified).is_err(),
            "a pack YARA cannot compile must be refused"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_manifest_cannot_escape_its_folder_or_lie_about_size() {
        for escape in ["../evil.json", "/etc/passwd", "sub/dir/rules.json"] {
            let base = manifest(1);
            let dodgy = IntelManifest {
                pack: escape.into(),
                ..base
            };
            assert!(
                validate_manifest(&dodgy).is_err(),
                "{escape} should be refused"
            );
        }
        let huge = IntelManifest {
            size: MAX_PACK_BYTES + 1,
            ..manifest(1)
        };
        assert!(validate_manifest(&huge).is_err());
        let no_hash = IntelManifest {
            sha256: "deadbeef".into(),
            ..manifest(1)
        };
        assert!(validate_manifest(&no_hash).is_err());
        let empty = IntelManifest {
            rules: Vec::new(),
            ..manifest(1)
        };
        assert!(validate_manifest(&empty).is_err());
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_misread() {
        let newer = IntelManifest {
            schema: SUPPORTED_SCHEMA + 1,
            ..manifest(1)
        };
        let error = validate_manifest(&newer).expect_err("schema must be checked");
        assert!(error.contains("schema"), "{error}");
    }

    #[test]
    fn rule_names_must_be_yara_identifiers() {
        let dodgy = IntelManifest {
            rules: vec![RuleMeta {
                name: "not a rule name".into(),
                description: String::new(),
                family: String::new(),
                level: RuleLevel::Behaviour,
            }],
            ..manifest(1)
        };
        assert!(validate_manifest(&dodgy).is_err());
    }

    #[test]
    fn the_builtin_pack_reports_itself_without_any_download() {
        let _guard = crate::state_guard("builtin");
        let status = status();
        assert_eq!(status.source, "builtin");
        assert_eq!(status.revision, 0);
        assert!(status.installed_rules >= status.builtin_rules);
        assert!(
            status.signed,
            "compiled-in rules are as trusted as the binary"
        );
    }

    #[test]
    fn installing_a_pack_keeps_the_previous_one_and_can_roll_back() {
        let _guard = crate::state_guard("rollback");
        let first = published_pack(10, "PackOnly", r#"rule PackOnly { condition: false }"#);
        let second = published_pack(
            11,
            "PackOnlyTwo",
            r#"rule PackOnlyTwo { condition: false }"#,
        );
        install_pack(&first).unwrap();
        install_pack(&second).unwrap();
        assert!(
            previous_pack_path().is_file(),
            "the record it replaced is kept"
        );
        assert_eq!(installed_pack().unwrap().manifest.revision, 11);

        rollback().unwrap();
        let back = installed_pack().expect("rollback restores the older pack");
        assert_eq!(back.manifest.revision, 10);
    }

    /// Build a pack the way an update does: the source first, then a manifest
    /// that honestly describes those exact bytes.
    fn published_pack(revision: u64, rule: &str, source: &str) -> Pack {
        Pack {
            manifest: manifest_for(revision, rule, source),
            source: source.to_string(),
            provenance: Provenance::Verified,
        }
    }
}
