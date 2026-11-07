//! The two published detectors: a SHA-256 blacklist and a YARA-X rule set.
//!
//! Both are compiled once per scan and shared across workers. The YARA set is
//! built into the binary so a scan works with the network off and before any
//! rule pack has been downloaded, and an external pack can add to it; the pack
//! format and its signature live in [`crate::intel`].
//!
//! What is *not* here matters as much as what is. Apple's XProtect signatures
//! are not redistributed — macOS applies them itself, and squid reports that
//! fact instead of borrowing Apple's authority. The built-in set is therefore
//! small, explicit, and self-contained: the EICAR test file, so "the pipeline
//! works" is testable on any machine, plus a handful of behaviour rules that
//! describe what the scanner can actually see in a startup item or a browser
//! extension. Every rule states the level it can support, and a behaviour rule
//! can only ever support `suspicious`.

use crate::findings::{Confidence, Evidence};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One compiled YARA rule plus the squid-side metadata YARA itself does not
/// carry: how much weight a match deserves.
pub struct Detector {
    pub rules: yara_x::Rules,
    /// Rule name → (description, family, level).
    pub meta: Vec<RuleMeta>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleMeta {
    pub name: String,
    pub description: String,
    pub family: String,
    pub level: RuleLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleLevel {
    /// A published signature for a named threat: enough to call it confirmed.
    Signature,
    /// A behaviour or structure pattern: worth a look, never a verdict.
    Behaviour,
}

/// The built-in source. Rule metadata is mirrored in `meta()` because YARA's
/// own `meta` block is typed loosely and squid wants the level to be a type.
const BUILTIN_YARA: &str = r#"
import "macho"
import "pe"

rule Eicar_Test_File
{
    meta:
        description = "The EICAR anti-malware test string"
        family = "EICAR-Test-File"
    strings:
        $a = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*" ascii wide
    condition:
        $a
}

rule ShellPayloadInUserData
{
    meta:
        description = "Shell commands that pipe a download straight into an interpreter"
        family = "behaviour"
    strings:
        $curl = /curl[ \t]+-s[^\n]{0,80}\|[ \t]*(ba)?sh/ ascii wide
        $wget = /wget[ \t]+-q[^\n]{0,80}\|[ \t]*(ba)?sh/ ascii wide
        $osascript = "osascript -e" ascii wide
        $eval = "eval $(curl" ascii wide
    condition:
        filesize < 2MB and 2 of them
}

rule LoginItemPointsAtTemp
{
    meta:
        description = "A launch plist whose program lives in a temporary directory"
        family = "behaviour"
    strings:
        $var = "/var/folders/" ascii wide
        $tmp = "/tmp/" ascii wide
        $prog1 = "<key>Program</key>" ascii wide
        $prog2 = "<key>ProgramArguments</key>" ascii wide
    condition:
        filesize < 256KB and ($prog1 or $prog2) and ($var or $tmp)
}

rule ExtensionHostsPermission
{
    meta:
        description = "A browser extension manifest that asks to read every site it visits"
        family = "behaviour"
    strings:
        $host = "\"<all_urls>\"" ascii wide
        $tabs = "\"tabs\"" ascii wide
        $debug = "\"debugger\"" ascii wide
        $name = "\"manifest_version\"" ascii wide
    condition:
        filesize < 64KB and $name and $host and ($tabs or $debug)
}

rule ScriptedOfficeMacro
{
    meta:
        description = "A document carrying an auto-run macro that starts a program"
        family = "behaviour"
    strings:
        $a = "AutoOpen" ascii wide
        $b = "AutoExec" ascii wide
        $c = "Shell(" ascii wide
        $d = "MacScript" ascii wide
    condition:
        filesize < 32MB and 3 of them
}

rule MachoDylibFromWritablePath
{
    meta:
        description = "A binary that loads a library from a folder the user can rewrite"
        family = "behaviour"
    condition:
        macho.number_of_segments > 0
        and for any d in macho.dylibs : (
            d.name matches /^\/(Users|tmp|private\/tmp|var\/folders)\//
        )
}

rule PeWithoutVersionInfo
{
    meta:
        description = "A Windows binary that names nobody in its version resource"
        family = "behaviour"
    condition:
        pe.number_of_sections > 1 and pe.number_of_version_infos == 0
}

rule PeUnnamedSection
{
    meta:
        description = "A Windows binary with a section packers leave nameless"
        family = "behaviour"
    condition:
        pe.number_of_sections > 0
        and for any s in pe.sections : ( s.name == "" )
}
"#;

/// The metadata table for the source above. A rule that matches is only as
/// strong as the level stated here, and behaviour rules can never reach
/// `confirmed` no matter how many of them fire at once.
pub fn builtin_meta() -> Vec<RuleMeta> {
    let behaviour = |name: &str, description: &str| RuleMeta {
        name: name.into(),
        description: description.into(),
        family: "behaviour".into(),
        level: RuleLevel::Behaviour,
    };
    vec![
        RuleMeta {
            name: "Eicar_Test_File".into(),
            description: "The standard anti-malware test string".into(),
            family: "EICAR-Test-File".into(),
            level: RuleLevel::Signature,
        },
        behaviour(
            "ShellPayloadInUserData",
            "Shell commands that pipe a download straight into an interpreter",
        ),
        behaviour(
            "LoginItemPointsAtTemp",
            "A launch plist whose program lives in a temporary directory",
        ),
        behaviour(
            "ExtensionHostsPermission",
            "A browser extension manifest that asks to read every site it visits",
        ),
        behaviour(
            "ScriptedOfficeMacro",
            "A document carrying an auto-run macro that starts a program",
        ),
        behaviour(
            "MachoDylibFromWritablePath",
            "A binary that loads a library from a folder the user can rewrite",
        ),
        behaviour(
            "PeWithoutVersionInfo",
            "A Windows binary that names nobody in its version resource",
        ),
        behaviour(
            "PeUnnamedSection",
            "A Windows binary with a section packers leave nameless",
        ),
    ]
}

impl Detector {
    /// Compile the built-in rules. A failure here is a build bug, not a runtime
    /// condition, so it surfaces as an error the caller reports rather than a
    /// silently empty detector.
    pub fn builtin() -> Result<Detector, String> {
        Self::from_source(BUILTIN_YARA, builtin_meta())
    }

    /// Built-in rules plus an external pack. If the pack fails to compile, the
    /// caller decides whether to fall back; this function does not hide that.
    pub fn with_extra(extra_sources: &[(String, Vec<RuleMeta>)]) -> Result<Detector, String> {
        let mut source = String::from(BUILTIN_YARA);
        let mut meta = builtin_meta();
        for (text, pack_meta) in extra_sources {
            source.push('\n');
            source.push_str(text);
            meta.extend(pack_meta.iter().cloned());
        }
        Self::from_source(&source, meta)
    }

    /// Compile a downloaded pack against the metadata the pack itself declares.
    /// Used by [`crate::intel`] as the acceptance test before a pack becomes
    /// `current`.
    pub fn from_source_for_pack(source: &str, meta: &[RuleMeta]) -> Result<Detector, String> {
        Self::from_source(source, meta.to_vec())
    }

    fn from_source(source: &str, meta: Vec<RuleMeta>) -> Result<Detector, String> {
        let rules = yara_x::compile(source).map_err(|error| format!("{error}"))?;
        Ok(Detector { rules, meta })
    }

    pub fn rule_count(&self) -> usize {
        self.rules.iter().count()
    }

    /// Scan one buffer. Returns evidence for every rule that fired, at the
    /// level its metadata allows.
    pub fn scan_buffer(&self, data: &[u8]) -> Vec<Evidence> {
        let mut scanner = yara_x::Scanner::new(&self.rules);
        let Ok(results) = scanner.scan(data) else {
            return Vec::new();
        };
        results
            .matching_rules()
            .filter_map(|matched| {
                let name = matched.identifier();
                let meta = self.meta.iter().find(|entry| entry.name == name)?;
                Some(Evidence::YaraRule {
                    rule: name.to_string(),
                    description: meta.description.clone(),
                    family: meta.family.clone(),
                })
            })
            .collect()
    }

    /// Level a matching rule can support. Rules with no metadata entry are
    /// treated as behaviour, which is the weaker answer.
    pub fn level_of(&self, rule: &str) -> RuleLevel {
        self.meta
            .iter()
            .find(|entry| entry.name == rule)
            .map(|entry| entry.level)
            .unwrap_or(RuleLevel::Behaviour)
    }

    pub fn confidence_for(&self, evidence: &[Evidence]) -> Confidence {
        evidence
            .iter()
            .filter_map(|item| match item {
                Evidence::YaraRule { rule, .. } => Some(self.level_of(rule)),
                _ => None,
            })
            .map(|level| match level {
                RuleLevel::Signature => Confidence::Confirmed,
                RuleLevel::Behaviour => Confidence::Suspicious,
            })
            .max()
            .unwrap_or(Confidence::Advisory)
    }

    /// Scan one file. Large files are mapped when possible, so a 4 GB disk
    /// image in Downloads does not become a 4 GB allocation, and a read failure
    /// is an empty answer rather than an error: unreadable files are already
    /// reported through coverage.
    pub fn scan_file(&self, path: &Path) -> Vec<Evidence> {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return Vec::new();
        };
        if !metadata.is_file() {
            return Vec::new();
        }
        // YARA needs the bytes in memory. Above this, only the head and tail
        // are offered, and the finding says nothing about the middle.
        const FULL_LIMIT: u64 = 64 * 1024 * 1024;
        const WINDOW: usize = 4 * 1024 * 1024;
        let read = |offset: u64, length: usize| -> Vec<u8> {
            use std::io::{Read, Seek, SeekFrom};
            let mut file = match std::fs::File::open(path) {
                Ok(file) => file,
                Err(_) => return Vec::new(),
            };
            if file.seek(SeekFrom::Start(offset)).is_err() {
                return Vec::new();
            }
            let mut buffer = Vec::with_capacity(length.min(16 * 1024 * 1024));
            let _ = file.by_ref().take(length as u64).read_to_end(&mut buffer);
            buffer
        };

        if metadata.len() <= FULL_LIMIT {
            return self.scan_buffer(&read(0, metadata.len() as usize));
        }
        let mut evidence = self.scan_buffer(&read(0, WINDOW));
        let tail = metadata.len().saturating_sub(WINDOW as u64);
        for item in self.scan_buffer(&read(tail, WINDOW)) {
            if !evidence.iter().any(|kept| kept.key() == item.key()) {
                evidence.push(item);
            }
        }
        evidence
    }
}

/// The SHA-256 blacklist. A hash match is the only detector that can call a
/// file confirmed on its own, because it says "this exact sequence of bytes was
/// published as bad".
#[derive(Clone, Debug, Default)]
pub struct HashIndex {
    entries: Vec<DeniedHash>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeniedHash {
    pub sha256: String,
    pub threat: String,
    #[serde(default)]
    pub family: String,
}

impl HashIndex {
    /// Built-in list. EICAR is here as well as in the rules, because the test
    /// file is how a person verifies that scanning works at all.
    pub fn builtin() -> HashIndex {
        HashIndex {
            entries: vec![
                // The test string as published (CRLF), as a text editor would
                // save it (LF), and bare. Which terminator survives a download
                // depends on the tool that wrote the file, and a test file that
                // matches only sometimes is worse than none: it makes a working
                // scanner look broken.
                DeniedHash {
                    sha256: "8b3f191819931d1f2cef7289239b5f77c00b079847b9c2636e56854d1e5eff71"
                        .into(),
                    threat: "EICAR test file".into(),
                    family: "EICAR-Test-File".into(),
                },
                DeniedHash {
                    sha256: "131f95c51cc819465fa1797f6ccacf9d494aaaff46fa3eac73ae63ffbdfd8267"
                        .into(),
                    threat: "EICAR test file".into(),
                    family: "EICAR-Test-File".into(),
                },
                DeniedHash {
                    sha256: "275a021bbfb6489e54d471899f7db9d1663fc695ec2fe2a2c4538aabf651fd0f"
                        .into(),
                    threat: "EICAR test file".into(),
                    family: "EICAR-Test-File".into(),
                },
            ],
        }
    }

    pub fn from_entries(entries: Vec<DeniedHash>) -> HashIndex {
        HashIndex { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn lookup(&self, sha256: &str) -> Option<&DeniedHash> {
        let needle = sha256.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|entry| entry.sha256.eq_ignore_ascii_case(&needle))
    }

    pub fn check(&self, sha256: &str) -> Option<Evidence> {
        self.lookup(sha256).map(|entry| Evidence::DeniedHash {
            sha256: entry.sha256.clone(),
            threat: entry.threat.clone(),
        })
    }
}

/// SHA-256 of a file, streamed. Returns `None` for anything that is not a
/// regular file or cannot be opened.
pub fn sha256_of(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let mut file = std::fs::File::open(path).ok()?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 128 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer).ok()?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Some(hex::encode(digest.finalize()))
}

/// The cheap half of the gate: can this name possibly be something that runs?
///
/// Used while walking, where only the path is free. It is deliberately
/// over-inclusive — an extensionless name always passes — because the answer it
/// avoids is expensive, and the file's `stat` data settles it a moment later in
/// [`worth_scanning`].
/// `assume_payload_location` widens the name gate for a scan the user pointed at
/// a folder of their own, where nothing about the location can be assumed.
pub fn plausible_by_name_in(path: &Path, assume_payload_location: bool) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy().to_ascii_lowercase();
    match name.rsplit_once('.') {
        // `thing.dylib`, `com.foo.plist`, and a dotted name whose tail is not an
        // extension at all (`v2ray-core`, `a.b.c`) all fall through to the
        // extensionless branch, which is the safe direction.
        Some((_, extension))
            if !extension.contains('.') && EXTENSION_GATED.contains(&extension) =>
        {
            true
        }
        Some(_) | None => assume_payload_location || extensionless_may_run(path),
    }
}

/// Extensions that are only ever worth a look where a payload is expected.
/// Plists are the launchd configuration itself, and are the single most common
/// place a persistence mechanism is written, so they are judged by name alone.
const EXTENSION_GATED: [&str; 3] = ["plist", "mobileconfig", "crx"];

/// Extensions that mean "this runs" anywhere on the machine.
///
/// PowerShell, batch and HTA are on the list because they are the most common way
/// a Windows machine is actually infected, and they are kilobytes of text each.
/// Not on it: `.js`, `.json`, `.html`, `.lnk`. Those are everywhere — 35,000 files
/// and 1.3 GB in the quick-scan range of an ordinary developer's Mac — and a scan
/// that reads all of them to find the occasional malicious one is a scan nobody
/// runs. A custom scan still sees them, because there the name is not trusted.
const EXECUTABLE_EXTENSIONS: [&str; 23] = [
    "dylib", "so", "xpc", "bundle", "exe", "dll", "sys", "ocx", "msi", "appx", "dmg", "pkg", "jar",
    "sh", "bash", "command", "scpt", "vbs", "ps1", "psm1", "bat", "cmd", "hta",
];

/// An extensionless name passes the cheap gate only where something extensionless
/// can plausibly live: inside an app bundle's own executable folder, in a
/// launchd directory, or in the folders a browser writes to. Everywhere else the
/// name alone cannot tell a Mach-O from a cache file, and `~/Library` holds
/// hundreds of thousands of the latter.
fn extensionless_may_run(path: &Path) -> bool {
    let location = layout_of(path);
    matches!(
        location,
        Layout::AppBundle | Layout::Launchd | Layout::DropZone | Layout::Browser
    )
}

/// Whether one file is worth opening, decided from the name and the `stat` data
/// the scan already has.
///
/// The gate has to be cheap *and* structural, because the folders a scan walks
/// hold about a million files and almost none of them can run. An extension list
/// alone fails in both directions: it misses the extensionless Mach-O a
/// dropper leaves in `~/Downloads`, and — taken the other way, as "extensionless
/// might be a binary" — it turns every cache, database and `*.dat` resource in
/// `~/Library` into a candidate, which is what made a full scan read the whole
/// disk before it found anything.
///
/// So the decision is made of three cheap questions, in this order: how big is it
/// (nothing under 64 bytes is a program, and a scan will not read a disk image),
/// can it be executed at all (the mode bits, which is what separates a real
/// binary from a cache file wearing no extension), and does the name or the header
/// say code. The header is read only for the few files the first two questions
/// leave ambiguous.
pub fn worth_scanning(path: &Path, metadata: &std::fs::Metadata) -> bool {
    worth_scanning_in(path, metadata, false)
}

/// `assume_payload_location` lifts the location test. A custom scan is asked
/// about the folder the user named, and a file they dropped there may be a
/// Mach-O with no extension, no execute bit and an unhelpful suffix, so the
/// header has to be consulted wherever the file happens to be.
pub fn worth_scanning_in(
    path: &Path,
    metadata: &std::fs::Metadata,
    assume_payload_location: bool,
) -> bool {
    if !metadata.is_file() {
        return false;
    }
    let size = metadata.len();
    if !(MIN_PROGRAM_BYTES..=SCAN_FILE_LIMIT).contains(&size) {
        return false;
    }
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if EXECUTABLE_EXTENSIONS.contains(&extension.as_str())
        || EXTENSION_GATED.contains(&extension.as_str())
    {
        return true;
    }
    // A folder the user named is asked about by content, not by name: what lands
    // there is theirs to ask about, a signature can sit in an ordinary text file,
    // and the suffix of a dropper's handiwork argues with itself. The size limits
    // above still apply, so the cost is bounded by the folder they chose.
    if assume_payload_location {
        return true;
    }
    // Elsewhere, a name that says what it is gets believed. `.svg`, `.class`,
    // `.zip` and `.docx` are not programs whatever the mode bits claim, and the
    // mode bits routinely *do* claim it: unpacking an archive marks everything
    // inside it executable, which is how a scan of a clean machine reported nine
    // hundred "unsigned binary" findings that were stylesheets and class files.
    if !extension.is_empty() {
        return false;
    }
    // No extension, so the execute bit is the only thing to go on. It separates
    // the few thousand launchable extensionless files in a home folder from the
    // hundreds of thousands of extensionless cache entries, and costs nothing.
    if is_executable(metadata) {
        return true;
    }
    // A browser can hand over a Mach-O with no extension and no execute bit, and
    // a dropper may run one through `/bin/sh`. In the folders where that happens
    // the first bytes are consulted, and only there.
    matches!(layout_of(path), Layout::DropZone | Layout::Browser) && looks_executable(path)
}

/// Below this a file cannot say anything about itself. `#!/bin/sh` plus one line
/// is the shortest payload that can actually run something, and it is a little
/// over twenty bytes; anything smaller is a stub, and the startup inventory
/// reports what points at it anyway.
const MIN_PROGRAM_BYTES: u64 = 24;

/// Where a path sits, in terms a scan cares about. Computed from the string
/// because it is asked for every walked entry and a `metadata` call per entry
/// would cost more than the walk itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Inside `Foo.app/Contents`, at any depth.
    AppBundle,
    /// A launchd directory, whose plists *are* the persistence mechanism.
    Launchd,
    /// Where a browser writes what it downloads.
    DropZone,
    /// A browser's own support folders, where an extension lives.
    Browser,
    /// Somewhere else entirely.
    Other,
}

pub fn layout_of(path: &Path) -> Layout {
    let text = path.to_string_lossy().to_ascii_lowercase();
    let home = squid_platform::startup::home_dir();
    let home = home.to_string_lossy().to_ascii_lowercase();
    if text.contains("/contents/") || text.ends_with(".app") {
        return Layout::AppBundle;
    }
    if text.contains("/library/launchagents/")
        || text.contains("/library/launchdaemons/")
        || text.ends_with("/library/launchagents")
        || text.ends_with("/library/launchdaemons")
    {
        return Layout::Launchd;
    }
    if text.starts_with(&format!("{home}/downloads"))
        || text.starts_with(&format!("{home}/desktop"))
    {
        return Layout::DropZone;
    }
    const BROWSERS: [&str; 6] = [
        "/google/chrome",
        "/google/chrome canary",
        "/microsoft/edge",
        "/mozilla/firefox",
        "/webkit/",
        "/extensions/",
    ];
    if BROWSERS.iter().any(|hint| text.contains(hint)) {
        return Layout::Browser;
    }
    Layout::Other
}

/// The execute bit, for any owner. On Windows the notion does not exist and the
/// read-only flag is the nearest thing, so the name and header carry the decision
/// there instead.
#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// The largest file a scan will read. Above this a file is reported by its size
/// and left alone: a disk image or a game archive is not what a startup payload
/// looks like, and reading one can cost more than the rest of the scan together.
pub const SCAN_FILE_LIMIT: u64 = 256 * 1024 * 1024;

/// Read the first bytes and decide whether anything could ever execute them.
fn looks_executable(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0_u8; 8];
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    if read < 4 {
        return false;
    }
    is_macho_magic(&head)
        || head.starts_with(b"MZ")
        || head.starts_with(b"#!")
        || head.starts_with(b"PK")
}

/// Mach-O in either byte order, 32-bit, 64-bit, or as a fat archive.
fn is_macho_magic(head: &[u8]) -> bool {
    const MACHOS: [[u8; 4]; 6] = [
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xbe, 0xba, 0xfe, 0xca],
    ];
    MACHOS.iter().any(|magic| head.starts_with(magic))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("squid-guard-{name}"));
        let mut file = std::fs::File::create(&path).expect("temp file");
        file.write_all(bytes).expect("write temp file");
        path
    }

    const EICAR: &str = "X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*\r\n";

    #[test]
    fn the_builtin_ruleset_compiles() {
        let detector = Detector::builtin().expect("built-in rules must compile");
        assert_eq!(detector.rule_count(), builtin_meta().len());
        for entry in builtin_meta() {
            assert!(
                detector
                    .rules
                    .iter()
                    .any(|rule| rule.identifier() == entry.name),
                "{} is declared in metadata but not compiled",
                entry.name
            );
        }
    }

    #[test]
    fn eicar_is_found_by_both_detectors() {
        let detector = Detector::builtin().unwrap();
        let path = temp_file("eicar", EICAR.as_bytes());
        let evidence = detector.scan_file(&path);
        assert!(
            evidence
                .iter()
                .any(|item| item.key() == "yara:Eicar_Test_File"),
            "the YARA rule did not fire: {evidence:?}"
        );
        let hash = sha256_of(&path).expect("hash");
        let index = HashIndex::builtin();
        let denied = index
            .check(&hash)
            .expect("the hash blacklist should know EICAR");
        assert!(matches!(denied, Evidence::DeniedHash { .. }));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_signature_rule_is_confirmed_and_a_behaviour_rule_is_not() {
        let detector = Detector::builtin().unwrap();
        assert_eq!(detector.level_of("Eicar_Test_File"), RuleLevel::Signature);
        assert_eq!(
            detector.level_of("ShellPayloadInUserData"),
            RuleLevel::Behaviour
        );
        assert_eq!(
            detector.confidence_for(&[Evidence::YaraRule {
                rule: "Eicar_Test_File".into(),
                description: String::new(),
                family: String::new(),
            }]),
            Confidence::Confirmed
        );
        assert_eq!(
            detector.confidence_for(&[Evidence::YaraRule {
                rule: "ShellPayloadInUserData".into(),
                description: String::new(),
                family: String::new(),
            }]),
            Confidence::Suspicious
        );
        // A rule nobody described is assumed to be the weak kind.
        assert_eq!(detector.level_of("Made_Up"), RuleLevel::Behaviour);
    }

    #[test]
    fn behaviour_rule_needs_evidence_not_a_single_string() {
        let detector = Detector::builtin().unwrap();
        let path = temp_file(
            "pipe-sh",
            b"#!/bin/sh\ncurl -sS https://example.test/install | sh\nwget -qO- https://example.test/x | bash\n",
        );
        let evidence = detector.scan_file(&path);
        assert!(
            evidence
                .iter()
                .any(|item| item.key() == "yara:ShellPayloadInUserData"),
            "two piped-shell lines should fire: {evidence:?}"
        );

        // One line alone is how documentation and install instructions read.
        let quiet = temp_file(
            "pipe-sh-quiet",
            b"curl -sS https://example.test/install | sh\n",
        );
        assert!(detector.scan_file(&quiet).is_empty());
        std::fs::remove_file(path).ok();
        std::fs::remove_file(quiet).ok();
    }

    #[test]
    fn a_clean_binary_reports_nothing() {
        let detector = Detector::builtin().unwrap();
        let path = temp_file("clean", b"\x7fELF this file is deliberately uninteresting");
        assert!(detector.scan_file(&path).is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn hashing_skips_directories_and_refusals() {
        assert!(sha256_of(std::path::Path::new("/")).is_none());
        assert!(sha256_of(std::path::Path::new("/definitely/not/here")).is_none());
    }

    #[test]
    fn only_executable_shaped_files_are_worth_reading() {
        // The gate is decided from real `stat` data, so the fixture has to be
        // real files: a cache file and a binary are the same name shape and
        // differ only in their mode bits.
        let dir = std::env::temp_dir().join("squid-gate");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let made = |name: &str, executable: bool| -> std::path::PathBuf {
            let path = dir.join(name);
            std::fs::write(&path, [0x90_u8; 512]).expect("write");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if executable { 0o755 } else { 0o644 };
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .expect("mode");
            }
            #[cfg(not(unix))]
            let _ = executable;
            path
        };

        let keep = |name: &str, executable: bool| {
            let path = made(name, executable);
            let metadata = std::fs::metadata(&path).expect("metadata");
            let kept = worth_scanning(&path, &metadata);
            std::fs::remove_file(&path).ok();
            kept
        };
        let keep_size = |name: &str, executable: bool, bytes: usize| {
            let path = dir.join(name);
            std::fs::write(&path, vec![0x90_u8; bytes]).expect("write");
            let metadata = std::fs::metadata(&path).expect("metadata");
            let kept = worth_scanning(&path, &metadata);
            std::fs::remove_file(&path).ok();
            let _ = executable;
            kept
        };

        assert!(keep("install.sh", false), "a script is a payload");
        assert!(keep("thing.dylib", false), "a library is a payload");
        assert!(
            keep("com.tester.agent.plist", false),
            "an agent is persistence"
        );
        assert!(!keep("notes.txt", false), "a text file is not a program");
        assert!(!keep("photo.jpg", false), "an image is not a program");
        assert!(
            !keep_size("stub", false, 8),
            "under the floor a file cannot say anything about itself"
        );
        // The execute bit is a POSIX notion; Windows has no equivalent, which is
        // why the name and the header carry the decision there instead.
        if cfg!(unix) {
            assert!(keep("runner", true), "an extensionless binary is a payload");
            assert!(
                !keep("runner-copy", false),
                "an extensionless file nobody can execute is data"
            );
            // Unpacking an archive marks everything inside it executable, so on a
            // machine with a few downloaded source trees the mode bit alone reports
            // hundreds of "unsigned binary" findings that are stylesheets. A name
            // that says what the file is beats a mode bit that does not.
            assert!(
                !keep("notes.txt", true),
                "an executable text file is still a text file"
            );
            assert!(
                !keep("styles.scss", true),
                "the exec bit does not turn a stylesheet into a payload"
            );
        }
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn a_folder_the_user_named_is_judged_by_content_not_by_name() {
        // A signature can live in an ordinary text file, and the suffix of a
        // dropper's handiwork argues with itself, so a scan pointed at one folder
        // asks about anything large enough to hold a payload. The same file found
        // while walking `~/Library` is taken at its word.
        let path = temp_file("payload", b"eicar-shaped text that a rule can match");
        let metadata = std::fs::metadata(&path).expect("metadata");
        assert!(
            worth_scanning_in(&path, &metadata, true),
            "inside a folder the user named, the name does not decide"
        );
        assert!(
            !worth_scanning(&path, &metadata),
            "the same file elsewhere is a text file, and a scan says so"
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn the_layouts_a_quick_scan_trusts_are_the_ones_that_can_run() {
        assert_eq!(
            layout_of(std::path::Path::new(
                "/Applications/Thing.app/Contents/MacOS/Thing"
            )),
            Layout::AppBundle
        );
        assert_eq!(
            layout_of(std::path::Path::new("/Library/LaunchAgents/x.plist")),
            Layout::Launchd
        );
        assert_eq!(
            layout_of(std::path::Path::new("/Applications/Other.txt")),
            Layout::Other,
            "/Applications on its own is not a bundle"
        );
        assert!(plausible_by_name_in(
            std::path::Path::new("/a/b/com.tester.agent.plist"),
            false
        ));
        assert!(!plausible_by_name_in(
            std::path::Path::new("/Users/tester/Documents/notes.txt"),
            false
        ));
    }

    #[test]
    fn an_extra_pack_adds_rules_without_losing_the_builtin_ones() {
        let extra = vec![(
            "rule PackProbe { strings: $a = \"pack-marker\" condition: $a }".to_string(),
            vec![RuleMeta {
                name: "PackProbe".into(),
                description: "from the pack".into(),
                family: "pack".into(),
                level: RuleLevel::Signature,
            }],
        )];
        let detector = Detector::with_extra(&extra).expect("the pack should compile");
        assert_eq!(detector.rule_count(), builtin_meta().len() + 1);
        assert_eq!(detector.level_of("PackProbe"), RuleLevel::Signature);
        assert!(detector
            .scan_buffer(b"prefix pack-marker suffix")
            .iter()
            .any(|item| item.key() == "yara:PackProbe"));
    }

    #[test]
    fn a_broken_pack_is_reported_not_swallowed() {
        let broken = vec![("rule {{{ not yara".to_string(), Vec::new())];
        assert!(Detector::with_extra(&broken).is_err());
    }
}
