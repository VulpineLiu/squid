//! User trust rules for findings that are not threats.
//!
//! The scanner is intentionally conservative: an unsigned login item is worth
//! telling a person about. A real machine also has hundreds of those from
//! ordinary software, so the answer cannot be to lower the detector until it
//! stops seeing them. It is to let the user name the publisher, folder or
//! advisory class they trust.
//!
//! Two limits are deliberate. A confirmed finding is never matched, and an
//! evidence-class rule only applies to advisory findings. Trust can quiet a
//! note; it cannot hide a published threat.

use crate::findings::{Confidence, Evidence, Finding};
use squid_core::paths;
use squid_core::settings::{AllowEntry, AllowKind};

/// Evidence classes that describe ordinary software rather than a published
/// threat. These are the only classes an evidence rule may quiet.
pub const TRUSTABLE_EVIDENCE: [&str; 4] = ["startup", "unsigned", "writable", "missing"];

pub fn evidence_kind(evidence: &Evidence) -> Option<&'static str> {
    let key = evidence.key();
    let kind = key.split(':').next()?;
    TRUSTABLE_EVIDENCE
        .iter()
        .copied()
        .find(|known| *known == kind)
}

pub fn is_allowlisted(finding: &Finding, allowlist: &[AllowEntry]) -> bool {
    if finding.confidence == Confidence::Confirmed {
        return false;
    }
    allowlist.iter().any(|entry| match entry.kind {
        AllowKind::Publisher => finding
            .signed
            .then_some(finding.publisher.as_deref())
            .flatten()
            .is_some_and(|publisher| publisher.trim().eq_ignore_ascii_case(entry.value.trim())),
        AllowKind::Path => finding.path.as_deref().is_some_and(|path| {
            let trusted = paths::expand(&entry.value);
            paths::same_path(&trusted, path) || paths::is_within(&trusted, path)
        }),
        AllowKind::Evidence => {
            finding.confidence == Confidence::Advisory
                && finding
                    .evidence
                    .iter()
                    .filter_map(evidence_kind)
                    .any(|kind| kind == entry.value.trim().to_ascii_lowercase())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn finding(confidence: Confidence) -> Finding {
        Finding {
            id: "scan#1".into(),
            confidence,
            path: Some(PathBuf::from(
                "/Applications/Example.app/Contents/MacOS/Example",
            )),
            display_path: Some("/Applications/Example.app".into()),
            name: "Example".into(),
            title: "Example".into(),
            description: String::new(),
            evidence: vec![Evidence::Unsigned {
                path: PathBuf::from("/Applications/Example.app/Contents/MacOS/Example"),
                note: None,
            }],
            size: 1,
            sha256: None,
            signed: true,
            publisher: Some("Example Software Ltd".into()),
            found_at_ms: 0,
            quarantined: false,
            ignored: false,
        }
    }

    fn rule(kind: AllowKind, value: &str) -> AllowEntry {
        AllowEntry {
            kind,
            value: value.into(),
        }
    }

    #[test]
    fn a_publisher_or_bundle_folder_quiets_an_advisory() {
        let finding = finding(Confidence::Advisory);
        assert!(is_allowlisted(
            &finding,
            &[rule(AllowKind::Publisher, "example software ltd")]
        ));
        assert!(is_allowlisted(
            &finding,
            &[rule(AllowKind::Path, "/Applications/Example.app")]
        ));
        assert!(is_allowlisted(
            &finding,
            &[rule(AllowKind::Evidence, "unsigned")]
        ));
        assert!(!is_allowlisted(
            &finding,
            &[rule(AllowKind::Path, "/Applications/Other.app")]
        ));
    }

    #[test]
    fn an_evidence_rule_never_hides_a_suspicious_or_confirmed_finding() {
        assert!(!is_allowlisted(
            &finding(Confidence::Suspicious),
            &[rule(AllowKind::Evidence, "unsigned")]
        ));
        assert!(!is_allowlisted(
            &finding(Confidence::Confirmed),
            &[
                rule(AllowKind::Evidence, "unsigned"),
                rule(AllowKind::Publisher, "Example Software Ltd"),
                rule(AllowKind::Path, "/Applications"),
            ]
        ));
    }

    #[test]
    fn a_publisher_rule_requires_a_signed_file() {
        let mut unsigned = finding(Confidence::Advisory);
        unsigned.signed = false;
        assert!(!is_allowlisted(
            &unsigned,
            &[rule(AllowKind::Publisher, "Example Software Ltd")]
        ));
    }
}
