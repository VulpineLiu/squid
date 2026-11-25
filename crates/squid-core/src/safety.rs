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

/// Windows additions to the exact-deny list. The AppData tree is the
/// Windows equivalent of `~/Library`: deleting the folder itself is how
/// you destroy a profile, deleting one cache inside it is the job.
#[cfg(windows)]
const WINDOWS_DENY_EXACT: &[&str] = &[
    "~/AppData",
    "~/AppData/Local",
    "~/AppData/LocalLow",
    "~/AppData/Roaming",
    "~/AppData/Local/Packages",
    "~/OneDrive",
    "~/Links",
    "~/Favorites",
    "~/Contacts",
    "~/Saved Games",
    "~/Searches",
];

/// System folders that exist per drive and must never be touched, no
/// matter which drive the rule was anchored on.
#[cfg(windows)]
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "$Recycle.Bin",
    "$Extend",
    "System Volume Information",
    "Recovery",
    "PerfLogs",
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
#[cfg(not(windows))]
fn outside_home_ok() -> Vec<PathBuf> {
    ["/Applications"].iter().map(PathBuf::from).collect()
}

/// Windows equivalent of `/Applications`: the machine-wide program data
/// folders whose *contents* an ordinary user may delete from (they own
/// the ACL), while the roots themselves are on the hard-deny list.
#[cfg(windows)]
fn outside_home_ok() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ["ProgramData", "PUBLIC"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .collect();
    out.push(PathBuf::from(r"C:\ProgramData"));
    out.push(PathBuf::from(r"C:\Users\Public"));
    out
}

/// System folders derived from the environment so the deny list follows
/// non-default installs (Windows on D:, relocated Program Files, …).
#[cfg(windows)]
fn windows_hard_deny() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "PUBLIC",
        "USERPROFILE",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .collect();
    out.extend(
        [
            r"C:\Windows",
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\Users",
            r"C:\ProgramData",
            r"C:\Users\Public",
        ]
        .iter()
        .map(PathBuf::from),
    );
    out
}

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
    /// Roots whose contents a non-admin run may still reach — on Unix
    /// `/Applications`, on Windows `%ProgramData%` and `%PUBLIC%`.
    pub outside_home_ok: Vec<PathBuf>,
    /// Accept *any* existing volume as an allowed anchor, not just system
    /// paths. The duplicate-file scan runs over whole drives by design — the
    /// files are the user's own, they were confirmed byte-identical, and
    /// deletion still goes through the recycle bin + undo path. Everything
    /// else about the policy (hard-deny roots, protected folders, symlink
    /// refusal) keeps its full teeth.
    pub all_volumes_ok: bool,
}

impl Default for SafetyPolicy {
    fn default() -> Self {
        // mut is only exercised on Windows; keep both targets warning-free.
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut deny_exact: Vec<PathBuf> = DENY_EXACT.iter().map(|p| paths::expand(p)).collect();
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut hard_deny: Vec<PathBuf> = HARD_DENY.iter().map(PathBuf::from).collect();
        #[cfg(windows)]
        {
            deny_exact.extend(WINDOWS_DENY_EXACT.iter().map(|p| paths::expand(p)));
            hard_deny.extend(windows_hard_deny());
        }
        Self {
            deny_exact,
            hard_deny,
            min_depth: 1,
            forbidden_mounts: Vec::new(),
            allow_system_paths: true,
            follow_symlinks: false,
            outside_home_ok: outside_home_ok(),
            all_volumes_ok: false,
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

    /// Let an unprivileged run reach files on any existing volume (the
    /// duplicate-file scan); see [`SafetyPolicy::all_volumes_ok`].
    pub fn with_all_volumes(mut self) -> Self {
        self.all_volumes_ok = true;
        self
    }

    /// True when `norm` sits on a drive/volume that exists right now. The
    /// scan-anchors-are-volumes rule keeps its teeth in the other checks:
    /// hard-deny roots, deny-exact folders and reserved names all run first.
    fn on_existing_volume(&self, norm: &Path) -> bool {
        #[cfg(windows)]
        {
            match norm.components().next() {
                Some(std::path::Component::Prefix(prefix)) => {
                    PathBuf::from(prefix.as_os_str()).exists()
                }
                _ => false,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = norm;
            true
        }
    }

    fn is_exact_denied(&self, path: &Path) -> bool {
        self.deny_exact.iter().any(|deny| {
            // Trailing glob star entries deny the directory itself only.
            let d = deny
                .to_string_lossy()
                .trim_end_matches('*')
                .trim_end_matches(['/', '\\'])
                .to_string();
            paths::same_path(path, Path::new(&d))
        })
    }

    fn is_hard_denied(&self, path: &Path) -> bool {
        if paths::is_root_like(path) {
            return true;
        }
        self.hard_deny
            .iter()
            .any(|deny| paths::same_path(path, deny))
    }

    /// Validate a removal target. `anchors` are the roots the rule claims to
    /// own; the target must live strictly below one of them.
    pub fn check(&self, path: &Path, anchors: &[PathBuf]) -> Result<()> {
        #[cfg(windows)]
        {
            let norm = paths::normalized(path);
            for comp in norm.components() {
                if let std::path::Component::Normal(name) = comp {
                    if WINDOWS_RESERVED_NAMES
                        .iter()
                        .any(|r| name.to_string_lossy().eq_ignore_ascii_case(r))
                    {
                        return Err(SquidError::safety(path, "Windows system folder"));
                    }
                }
            }
        }
        if self.is_hard_denied(path) {
            return Err(SquidError::safety(path, "protected system location"));
        }
        if self.is_exact_denied(path) {
            return Err(SquidError::safety(path, "protected user data folder"));
        }
        for mount in &self.forbidden_mounts {
            if paths::same_path(path, mount) {
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
            && !(self.all_volumes_ok && self.on_existing_volume(&norm))
            && !paths::is_within(&paths::home_dir(), &norm)
            && !self
                .outside_home_ok
                .iter()
                .any(|allowed| paths::is_within(allowed, &norm))
        {
            return Err(SquidError::safety(
                path,
                format!(
                    "outside the current user's home ({} to clean system paths)",
                    crate::elevation_hint()
                ),
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
                    if !paths::same_path(&canonical, &expected) {
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
        //
        // The shared install folder is `/Applications` on Unix and the
        // machine-wide program-data folders on Windows, so the fixture is read
        // back from the policy instead of being hardcoded to one OS's spelling
        // — a literal `/Applications` on Windows is not even an absolute path,
        // and the test would have been asserting that Windows refuses an app
        // folder it never actually looked at.
        let policy = SafetyPolicy::user_only();
        let shared = policy
            .outside_home_ok
            .first()
            .cloned()
            .expect("every platform names a shared install folder");
        let app = if cfg!(windows) {
            shared.join("Demo")
        } else {
            shared.join("Demo.app")
        };
        let anchors = vec![shared.clone()];
        assert!(
            policy.check(&app, &anchors).is_ok(),
            "{app:?} has to be removable without elevation"
        );
        assert!(
            policy.denies(&shared, &anchors).is_some(),
            "the folder itself stays untouchable"
        );
        assert!(policy.denies(Path::new(system_file()), &anchors).is_some());
    }

    /// A file no run of squid may ever delete, on any platform.
    #[cfg(not(windows))]
    fn system_file() -> &'static str {
        "/System/Applications/Notes.app"
    }

    #[cfg(windows)]
    fn system_file() -> &'static str {
        r"C:\Windows\System32\drivers\ndis.sys"
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
