//! The last line of defence before anything is unlinked.
//!
//! Rules decide *what* to delete; this module decides whether deleting it
//! could turn into a data-loss incident. Every recursive removal goes through
//! [`SafetyPolicy::check`], including removals requested by the TUI.

use crate::error::{Result, SquidError};
use crate::paths;
use std::path::{Path, PathBuf};

/// Locations that must never be removed, no matter what a rule says.
const HARD_DENY: &[&str] = &[
    "/",
    "/bin",
    "/sbin",
    "/usr",
    "/etc",
    "/System",
    "/Library",
    "/Applications",
    "/Users",
    "/home",
    "/root",
    "/opt",
    "/private",
    "/private/etc",
    "/private/var",
    "/private/var/db",
    "/private/var/folders",
    "/Volumes",
    "/tmp",
    "/private/tmp",
    "/nix",
    "/snap",
    "/mnt",
    "/media",
    "/boot",
    "/proc",
    "/sys",
    "/dev",
    "/run",
    "/Program Files",
    "/Program Files (x86)",
    "/Windows",
    "/System/Volumes",
    "/System/Volumes/Data",
    "/private/var/root",
];

/// Prefixes that are dangerous to remove *recursively* but fine one level in.
const DENY_EXACT: &[&str] = &[
    "~/Library",
    "~/Library/Application Support",
    "~/Library/Containers",
    "~/Library/Group Containers",
    "~/Library/Caches",
    "~/Library/Preferences",
    "~/Library/Saved Application State",
    "~/Library/Mobile Documents",
    "~/Library/Mail",
    "~/Library/Messages",
    "~/Library/Photos",
    "~/Library/Keychains",
    "~/Library/Application Support/MobileSync",
    "~/Library/Application Support/CloudDocs",
    "~/Library/Application Support/com.apple.icloud*",
    "~/Documents",
    "~/Desktop",
    "~/Downloads",
    "~/Pictures",
    "~/Music",
    "~/Movies",
    "~/.ssh",
    "~/.gnupg",
    "~/.aws",
    "~/.config/gcloud",
    "~/.docker",
    "~/.subversion",
    "~/.zsh_history",
    "~/.bash_history",
];

/// Folders outside the current user's home that a normal (non-root) user is
/// still allowed to remove *entries from*.
///
/// `SafetyPolicy::user_only` exists so that a run without administrator rights
/// cannot wander into system space, but read literally it also refuses the one
/// thing every uninstaller must do: move an app out of `/Applications`. An
/// admin-group user can do exactly that in Finder without a password, so the
/// shared application folder is carved out here. Only its *contents* are
/// reachable — `/Applications` itself is on the hard-deny list — and a bundle
/// owned by root is still locked much earlier by `writable_by_us`.
const OUTSIDE_HOME_OK: &[&str] = &["/Applications"];

#[derive(Clone, Debug)]
pub struct SafetyPolicy {
    deny_exact: Vec<PathBuf>,
    hard_deny: Vec<PathBuf>,
    /// Minimum path components below an allowed anchor for a recursive delete.
    pub min_depth: usize,
    /// Volumes (mount points) that must never be walked.
    pub forbidden_mounts: Vec<PathBuf>,
    /// When false, refuse anything outside the current user's home.
    pub allow_system_paths: bool,
    /// Never descend into a symlink (default, and the right answer).
    pub follow_symlinks: bool,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        Self {
            deny_exact: DENY_EXACT.iter().map(|p| paths::expand(p)).collect(),
            hard_deny: HARD_DENY.iter().map(PathBuf::from).collect(),
            min_depth: 1,
            forbidden_mounts: Vec::new(),
            allow_system_paths: true,
            follow_symlinks: false,
        }
    }
}

impl SafetyPolicy {
    pub fn system() -> Self {
        // Running as root: `/private/var/folders` and `/Library/Caches` become
        // reachable, but the deny list keeps its teeth.
        Self {
            allow_system_paths: true,
            ..Self::default()
        }
    }

    pub fn user_only() -> Self {
        Self {
            allow_system_paths: false,
            ..Self::default()
        }
    }

    pub fn with_mounts(mut self, mounts: Vec<PathBuf>) -> Self {
        self.forbidden_mounts = mounts;
        self
    }

    fn is_exact_denied(&self, path: &Path) -> bool {
        let norm = paths::normalized(path);
        self.deny_exact.iter().any(|deny| {
            let d = paths::normalized(deny);
            // Trailing glob star entries deny the directory itself only.
            let d = d
                .to_string_lossy()
                .trim_end_matches('*')
                .trim_end_matches('/')
                .to_string();
            norm == Path::new(&d)
        })
    }

    fn is_hard_denied(&self, path: &Path) -> bool {
        let norm = paths::normalized(path);
        if norm.as_os_str().is_empty() {
            return true;
        }
        self.hard_deny
            .iter()
            .any(|deny| norm == *deny || norm == Path::new("/"))
    }

    /// Validate a removal target. `anchors` are the roots the rule claims to
    /// own; the target must live strictly below one of them.
    pub fn check(&self, path: &Path, anchors: &[PathBuf]) -> Result<()> {
        if self.is_hard_denied(path) {
            return Err(SquidError::safety(path, "protected system location"));
        }
        if self.is_exact_denied(path) {
            return Err(SquidError::safety(path, "protected user data folder"));
        }
        for mount in &self.forbidden_mounts {
            if paths::normalized(path) == paths::normalized(mount) {
                return Err(SquidError::safety(path, "volume root"));
            }
        }

        let norm = paths::normalized(path);
        if let Some(anchor) = anchors.iter().find(|a| paths::is_within(a, &norm)) {
            let depth = paths::depth_below(anchor, &norm).unwrap_or(0);
            if depth < self.min_depth {
                return Err(SquidError::safety(
                    path,
                    format!(
                        "only {depth} level(s) below protected anchor {}",
                        anchor.display()
                    ),
                ));
            }
        } else {
            return Err(SquidError::safety(
                path,
                "outside every declared cleanup anchor",
            ));
        }

        if !self.allow_system_paths
            && !paths::is_within(&paths::home_dir(), &norm)
            && !OUTSIDE_HOME_OK
                .iter()
                .any(|allowed| paths::is_within(Path::new(allowed), &norm))
        {
            return Err(SquidError::safety(
                path,
                "outside the current user's home (use sudo to clean system paths)",
            ));
        }

        if !self.follow_symlinks {
            if std::fs::symlink_metadata(path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(SquidError::safety(path, "target is a symbolic link"));
            }
            if let Some(parent) = norm.parent() {
                let canonical = std::fs::canonicalize(parent).ok();
                if let Some(canonical) = canonical {
                    let expected = paths::normalized(parent);
                    if canonical != expected {
                        return Err(SquidError::safety(
                            path,
                            format!(
                                "parent resolves through a symlink to {}",
                                canonical.display()
                            ),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Convenience wrapper that reports violations instead of returning them.
    pub fn denies(&self, path: &Path, anchors: &[PathBuf]) -> Option<String> {
        self.check(path, anchors).err().map(|e| match e {
            SquidError::Safety { reason, .. } => reason,
            other => other.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> SafetyPolicy {
        SafetyPolicy {
            allow_system_paths: true,
            ..SafetyPolicy::default()
        }
    }

    #[test]
    fn refuses_roots_and_protected_folders() {
        let p = policy();
        let anchors = vec![PathBuf::from("/")];
        assert!(p.check(Path::new("/"), &anchors).is_err());
        assert!(p
            .check(Path::new("/System/Volumes/Data"), &anchors)
            .is_err());
        assert!(p.check(Path::new("/Users"), &anchors).is_err());
    }

    #[test]
    fn allows_deep_cache_paths() {
        let p = policy();
        let anchors = vec![PathBuf::from("/Users/tester/Library/Caches")];
        assert!(p
            .check(
                &PathBuf::from("/Users/tester/Library/Caches/com.example.app"),
                &anchors
            )
            .is_ok());
    }

    #[test]
    fn refuses_anchor_itself() {
        let p = policy();
        let anchors = vec![PathBuf::from("/Users/tester/Library/Caches")];
        assert!(p
            .check(&PathBuf::from("/Users/tester/Library/Caches"), &anchors)
            .is_err());
    }

    #[test]
    fn a_plain_user_can_still_uninstall_into_applications() {
        // The bug: as a normal user the home-only rule refused every bundle,
        // so `squid uninstall` approved the app and then deleted only its
        // leftovers.
        let policy = SafetyPolicy::user_only();
        let anchors = vec![PathBuf::from("/Applications")];
        assert!(policy
            .check(Path::new("/Applications/Demo.app"), &anchors)
            .is_ok());
        assert!(
            policy
                .denies(Path::new("/Applications"), &anchors)
                .is_some(),
            "the folder itself stays untouchable"
        );
        assert!(policy
            .denies(Path::new("/Library/LaunchDaemons/x.plist"), &anchors)
            .is_some());
        assert!(policy
            .denies(Path::new("/System/Applications/Notes.app"), &anchors)
            .is_some());
    }

    #[test]
    fn refuses_paths_outside_anchors() {
        let p = policy();
        let anchors = vec![PathBuf::from("/Users/tester/Library/Caches")];
        assert!(p
            .check(&PathBuf::from("/Users/tester/Documents/x"), &anchors)
            .is_err());
    }
}
