//! Judgement calls that no published signature makes for you.
//!
//! These rules describe conditions a person would want to know about, not
//! threats. Each one produces [`Evidence`] and a ceiling, and the ceiling is the
//! important part: heuristics stop at `suspicious`, so a machine with an
//! unsigned updater in `~/Library/Application Support` gets a row to read, not a
//! file moved behind its back.
//!
//! Two properties do most of the work. **Who can rewrite the file**: a binary
//! that starts at login from a folder the current user owns can be replaced by
//! anything the current user runs, which is how persistence malware survives an
//! update. **What claims to be true**: a file named like a system component, or
//! launched by an entry whose target has vanished, is a mismatch between the
//! story and the facts.

use crate::findings::{Confidence, Evidence};
use squid_platform::startup::{StartupItem, StartupRisk, StartupScope, StartupSource};
use std::path::{Path, PathBuf};

/// What the scanner knows about one candidate file when it asks for a judgement.
pub struct Candidate<'a> {
    pub path: &'a Path,
    /// The login/startup entry that launches this file, when there is one.
    pub startup: Option<&'a StartupItem>,
    /// True when any user of this machine can replace the file.
    pub world_writable: bool,
    /// Whether the file carries a signature blob, from
    /// [`squid_platform::trust::has_signature_blob`]. The caller answers this
    /// because it decides what the question is worth: asking the operating
    /// system to *validate* a signature costs a subprocess on macOS and a
    /// certificate-chain walk on Windows, which is affordable for the few files
    /// a scan is already reporting and ruinous for the thousands it is only
    /// filtering.
    pub signed: bool,
}

/// Judge one candidate. Returns evidence; the caller combines it with whatever
/// the published detectors found and takes the strongest level.
pub fn judge(candidate: &Candidate) -> Vec<Evidence> {
    let mut evidence = Vec::new();

    if let Some(item) = candidate.startup {
        evidence.push(Evidence::StartsAtLogin {
            entry: item.name.clone(),
            source: item.source.as_str().to_string(),
        });
        if item.risks.contains(&StartupRisk::MissingTarget) {
            evidence.push(Evidence::MissingTarget {
                path: item
                    .program
                    .clone()
                    .unwrap_or_else(|| candidate.path.to_path_buf()),
            });
        }
        if item.scope == StartupScope::System {
            // A system-scope entry pointed at a user-writable file is the
            // classic privilege hop: root runs something an unqualified user may
            // overwrite.
            if candidate.world_writable || starts_in_user_space(candidate.path) {
                evidence.push(Evidence::WritableLocation {
                    path: candidate.path.to_path_buf(),
                });
            }
        }
    }

    if !candidate.signed {
        evidence.push(Evidence::Unsigned {
            path: candidate.path.to_path_buf(),
            note: None,
        });
    }

    if starts_in_user_space(candidate.path) && candidate.startup.is_some() {
        evidence.push(Evidence::WritableLocation {
            path: candidate.path.to_path_buf(),
        });
    }

    if let Some(claimed) =
        impersonated_component(candidate.path).or_else(|| windows_lookalike(candidate.path))
    {
        evidence.push(Evidence::Impersonation {
            claimed: claimed.to_string(),
            path: candidate.path.to_path_buf(),
        });
    }

    dedupe(&mut evidence);
    evidence
}

/// The ceiling these heuristics can support: suspicious, never confirmed.
pub const CEILING: Confidence = Confidence::Suspicious;

/// Level for a set of heuristic findings alone.
///
/// One weak signal is a note; two that point the same way are a finding. A binary
/// with no signature is ordinary — squid itself is one — but a binary with no
/// signature *that starts at login* is the shape persistence malware takes, and a
/// file that claims to be a system component while sitting somewhere else is
/// suspicious on its own because nothing honest does that.
///
/// The alternative, which this used to be, was to call any single signal
/// suspicious. On a real machine that produced nine hundred rows next to a
/// download folder, and a list that long is read as noise and then ignored, which
/// is worse than not reporting it.
pub fn confidence(evidence: &[Evidence]) -> Confidence {
    let mut weight = 0_u8;
    for item in evidence {
        match item {
            // The one signal strong enough to stand alone.
            Evidence::Impersonation { .. } => return Confidence::Suspicious,
            Evidence::StartsAtLogin { .. }
            | Evidence::Unsigned { .. }
            | Evidence::WritableLocation { .. } => weight += 1,
            // "The file it points at is gone" is housekeeping. It explains a
            // finding and is worth showing beside one, but on its own it has
            // never hurt anybody.
            Evidence::MissingTarget { .. } => {}
            // Everything else is not a heuristic's verdict at all. A denied hash
            // or a YARA match is graded by the detector that produced it, and
            // counting it here would let a published hit lend false
            // corroboration to a weak heuristic sitting next to it.
            _ => {}
        }
    }
    if weight >= 2 {
        Confidence::Suspicious
    } else {
        Confidence::Advisory
    }
}

/// Locations every user of this machine can write to, and therefore locations
/// where a file that runs at login can be swapped without asking anybody.
fn starts_in_user_space(path: &Path) -> bool {
    let home = squid_platform::startup::home_dir();
    let text = path.to_string_lossy().to_lowercase();
    if path.starts_with(&home) {
        return true;
    }
    const WRITABLE: [&str; 4] = ["/users/shared/", "/tmp/", "/private/tmp/", "/var/folders/"];
    WRITABLE.iter().any(|prefix| text.starts_with(prefix))
}

/// Names that belong to the operating system, so a file wearing one somewhere
/// else is worth a look. Matched on the whole stem, never a substring, because
/// `SpotlightHelper.app` in a legit vendor's folder is not claiming to be Apple's.
const LOOKALIKES: [(&str, &str); 8] = [
    (
        "Activity Monitor",
        "/System/Applications/Utilities/Activity Monitor.app",
    ),
    ("Dock", "/System/Library/CoreServices/Dock.app"),
    (
        "ControlCenter",
        "/System/Library/CoreServices/ControlCenter.app",
    ),
    (
        "WindowServer",
        "/System/Library/PrivateFrameworks/SkyLight.framework",
    ),
    ("kernel_task", "the kernel"),
    ("launchd", "/sbin/launchd"),
    (
        "loginwindow",
        "/System/Library/CoreServices/loginwindow.app",
    ),
    (
        "UserNotificationCenter",
        "/System/Library/CoreServices/UserNotificationCenter.app",
    ),
];

fn impersonated_component(path: &Path) -> Option<&'static str> {
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stem = file_name
        .trim_end_matches(".app")
        .trim_end_matches(".plist")
        .to_string();
    let (name, real_home) = LOOKALIKES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&stem))?;
    // Apple's own copy living in its own place is not impersonating anything,
    // and neither is anything Apple ships inside /System.
    if path.starts_with("/System") || path.starts_with("/usr/libexec") {
        return None;
    }
    let _ = real_home;
    Some(*name)
}

/// Windows keeps its own set of names that people recognise and malware borrows.
#[cfg(windows)]
const WINDOWS_LOOKALIKES: [(&str, &str); 4] = [
    ("svchost", "%SystemRoot%\\System32\\svchost.exe"),
    ("explorer", "%SystemRoot%\\explorer.exe"),
    ("lsass", "%SystemRoot%\\System32\\lsass.exe"),
    ("taskhostw", "%SystemRoot%\\System32\\taskhostw.exe"),
];

#[cfg(windows)]
fn windows_lookalike(path: &Path) -> Option<&'static str> {
    let file_name = path.file_name()?.to_string_lossy().into_owned();
    let stem = file_name.trim_end_matches(".exe").to_string();
    let system = std::env::var("SystemRoot").unwrap_or_default();
    let (name, real) = WINDOWS_LOOKALIKES
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&stem))?;
    if !system.is_empty() && path.starts_with(&system) {
        return None;
    }
    let _ = real;
    Some(*name)
}

#[cfg(not(windows))]
fn windows_lookalike(_path: &Path) -> Option<&'static str> {
    None
}

pub fn dedupe(evidence: &mut Vec<Evidence>) {
    let mut seen: Vec<String> = Vec::with_capacity(evidence.len());
    evidence.retain(|item| {
        let key = item.key();
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
}

/// Startup entries whose own risk flags deserve a row in the findings list, even
/// though there is no file to quarantine. These are advisories about the
/// machine's persistence, and they are how a person finds an entry whose target
/// was deleted by hand and left behind.
pub fn startup_advisories(
    items: &[StartupItem],
) -> Vec<(PathBuf, String, Confidence, Vec<Evidence>)> {
    let mut out = Vec::new();
    for item in items {
        if item.system_vendor {
            continue;
        }
        let mut evidence = Vec::new();
        if item.risks.contains(&StartupRisk::MissingTarget) {
            evidence.push(Evidence::MissingTarget {
                path: item.program.clone().unwrap_or_default(),
            });
        }
        if item.risks.contains(&StartupRisk::InTempDir)
            || item.risks.contains(&StartupRisk::HiddenHomeDirectory)
        {
            evidence.push(Evidence::WritableLocation {
                path: item.program.clone().unwrap_or_default(),
            });
        }
        if item.risks.contains(&StartupRisk::ScriptInterpreter) {
            evidence.push(Evidence::StartsAtLogin {
                entry: item.name.clone(),
                source: format!("{} via script interpreter", item.source.as_str()),
            });
        }
        if item.source == StartupSource::LoginItem {
            continue;
        }
        if evidence.is_empty() {
            continue;
        }
        let level = if item.risks.contains(&StartupRisk::MissingTarget) {
            Confidence::Advisory
        } else {
            Confidence::Suspicious
        };
        out.push((
            item.program
                .clone()
                .unwrap_or_else(|| PathBuf::from(&item.id)),
            item.name.clone(),
            level,
            evidence,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use squid_platform::startup::{StartupScope, StartupSource};

    fn item(path: &Path, source: StartupSource, scope: StartupScope) -> StartupItem {
        let mut item = StartupItem::new(path.to_string_lossy(), "probe", source, scope);
        item.program = Some(path.to_path_buf());
        item
    }

    fn candidate<'a>(path: &'a Path, startup: Option<&'a StartupItem>) -> Candidate<'a> {
        Candidate {
            path,
            startup,
            world_writable: false,
            // The probes below are plain text written by the test, so this is
            // the honest answer for those files rather than a stub.
            signed: false,
        }
    }

    #[test]
    fn a_home_folder_file_is_user_writable() {
        let home = squid_platform::startup::home_dir();
        assert!(starts_in_user_space(
            &home.join("Library/Application Support/Thing/thing")
        ));
        assert!(starts_in_user_space(Path::new("/tmp/thing")));
        assert!(!starts_in_user_space(Path::new("/usr/libexec/thing")));
    }

    #[test]
    fn an_app_named_like_a_system_component_outside_its_home_is_flagged() {
        assert_eq!(
            impersonated_component(Path::new(
                "/Users/tester/Library/Application Support/Dock/Dock.app"
            )),
            Some("Dock")
        );
        // The real one is not impersonating itself.
        assert_eq!(
            impersonated_component(Path::new("/System/Library/CoreServices/Dock.app")),
            None
        );
        // A vendor app whose name merely contains the word is not a claim.
        assert_eq!(
            impersonated_component(Path::new("/Applications/DockFinder.app")),
            None
        );
    }

    #[test]
    fn heuristics_never_claim_confirmation() {
        let path = squid_platform::startup::home_dir()
            .join("Library/Application Support/guard-probe/updater");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"#!/bin/sh\necho probe\n").unwrap();
        let entry = item(&path, StartupSource::LaunchAgent, StartupScope::User);
        let evidence = judge(&candidate(&path, Some(&entry)));
        assert!(
            !evidence.is_empty(),
            "an unsigned login agent should say something"
        );
        assert_eq!(confidence(&evidence), Confidence::Suspicious);
        assert!(confidence(&evidence) != Confidence::Confirmed);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_lone_missing_target_is_housekeeping_not_a_threat() {
        let evidence = vec![
            Evidence::StartsAtLogin {
                entry: "ghost".into(),
                source: "launch-agent".into(),
            },
            Evidence::MissingTarget {
                path: PathBuf::from("/Users/tester/gone"),
            },
        ];
        assert_eq!(confidence(&evidence), Confidence::Advisory);
    }

    #[test]
    fn one_weak_signal_is_a_note_and_two_are_a_finding() {
        let unsigned = || Evidence::Unsigned {
            path: PathBuf::from("/a"),
            note: None,
        };
        let login = || Evidence::StartsAtLogin {
            entry: "thing".into(),
            source: "launch-agent".into(),
        };
        // The whole machine is unsigned. Saying so once per file is noise.
        assert_eq!(confidence(&[unsigned()]), Confidence::Advisory);
        assert_eq!(confidence(&[]), Confidence::Advisory);
        // Unsigned *and* started at login is the persistence shape.
        assert_eq!(
            confidence(&[unsigned(), login()]),
            Confidence::Suspicious,
            "corroborating signals have to outrank one lonely signal"
        );
        // Wearing a system component's name is enough on its own.
        assert_eq!(
            confidence(&[Evidence::Impersonation {
                claimed: "Dock".into(),
                path: PathBuf::from("/Users/tester/Dock"),
            }]),
            Confidence::Suspicious
        );
        // …and heuristics still cannot reach confirmed, whatever the combination.
        assert_ne!(
            confidence(&[unsigned(), login()]),
            Confidence::Confirmed,
            "no heuristic may claim certainty"
        );
    }

    #[test]
    fn a_published_hit_does_not_make_a_weak_heuristic_look_stronger() {
        // One unsigned file plus one confirmed hash hit. The hash decides the
        // level; the heuristic half must still say "one weak signal", or the
        // ceiling that keeps heuristics out of `confirmed` would be grading the
        // detector's evidence rather than its own.
        let evidence = vec![
            Evidence::DeniedHash {
                sha256: "aa".into(),
                threat: "known bad".into(),
            },
            Evidence::Unsigned {
                path: PathBuf::from("/a"),
                note: None,
            },
        ];
        assert_eq!(confidence(&evidence), Confidence::Advisory);
    }

    #[test]
    fn evidence_is_not_reported_twice() {
        let mut evidence = vec![
            Evidence::Unsigned {
                path: PathBuf::from("/a"),
                note: None,
            },
            Evidence::Unsigned {
                path: PathBuf::from("/a"),
                note: None,
            },
            Evidence::Unsigned {
                path: PathBuf::from("/b"),
                note: None,
            },
        ];
        dedupe(&mut evidence);
        assert_eq!(evidence.len(), 2);
    }
}
