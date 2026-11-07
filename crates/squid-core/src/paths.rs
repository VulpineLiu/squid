//! Path expansion helpers shared by the rule engine and the UI.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Expand a leading `~` / `$HOME` (also `~\` on Windows) and stop there.
///
/// Rules are authored with `~` because the crate ships a data file, not
/// hard-coded absolute paths, and the same rule file must work for the
/// current user *and* for `sudo` runs.
pub fn expand(input: &str) -> PathBuf {
    let home = home_dir();
    expand_with(input, &home)
}

pub fn expand_with(input: &str, home: &Path) -> PathBuf {
    let input = input.trim();
    if input == "~" || input == "$HOME" {
        return home.to_path_buf();
    }
    for prefix in ["~/", "~\\"] {
        if let Some(rest) = input.strip_prefix(prefix) {
            return home.join(rest.trim_start_matches(['/', '\\']));
        }
    }
    if let Some(rest) = input.strip_prefix("$HOME/") {
        return home.join(rest);
    }
    if let Some(rest) = input.strip_prefix("$HOME") {
        return home.join(rest.trim_start_matches(['/', '\\']));
    }
    PathBuf::from(input)
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Best-effort `/Volumes/...`-aware display shortening for the UI.
pub fn pretty(path: &Path) -> String {
    let home = home_dir();
    if same_path(&home, path) {
        return "~".to_string();
    }
    match relative_below(&home, path) {
        Some(rel) => {
            // Windows renders the tail with `\`, so `~/AppData\Roaming` reads
            // like a typo. Follow the separator the platform itself uses.
            let sep = if cfg!(windows) { '\\' } else { '/' };
            format!("~{sep}{}", rel.display())
        }
        None => path.display().to_string(),
    }
}

/// Strip the `\\?\` / `\??\` verbatim prefix `canonicalize` prepends on
/// Windows so canonical paths can be compared with hand-built ones.
#[cfg(windows)]
fn strip_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if text.starts_with(r"\\?\") || text.starts_with(r"\??\") {
        return PathBuf::from(text[4..].to_string());
    }
    path.to_path_buf()
}

#[cfg(not(windows))]
fn strip_verbatim(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Comparable spelling plus original text for every component of a path.
///
/// Windows path lookup is case-insensitive and `canonicalize` hands back
/// verbatim `\\?\C:\…` paths whose drive letter casing can differ from a
/// hand-built path, so all containment/equality decisions funnel through
/// these lower-cased keys (with the drive root's `RootDir` folded into the
/// `C:` prefix). On Unix the key is the literal component.
fn parts(path: &Path) -> Vec<(String, OsString)> {
    normalized(&strip_verbatim(path))
        .components()
        .filter_map(|c| match c {
            Component::RootDir => {
                if cfg!(windows) {
                    None
                } else {
                    Some(("/".to_string(), OsString::from("/")))
                }
            }
            other => {
                let text = other.as_os_str().to_string_lossy().into_owned();
                Some((text.to_lowercase(), other.as_os_str().to_os_string()))
            }
        })
        .collect()
}

fn keys_match(prefix: &[(String, OsString)], path: &[(String, OsString)]) -> bool {
    path.len() >= prefix.len()
        && prefix
            .iter()
            .zip(path)
            .all(|(a, b)| a.0.eq_ignore_ascii_case(&b.0))
}

/// True when `child` is strictly inside `parent`.
///
/// Comparison follows platform semantics: case-insensitive on Windows.
pub fn is_within(parent: &Path, child: &Path) -> bool {
    let (p, c) = (parts(parent), parts(child));
    c.len() > p.len() && keys_match(&p, &c)
}

/// Path equality under platform semantics: case-insensitive on Windows,
/// verbatim spellings folded, `C:\` equal to `C:`.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (parts(a), parts(b));
    a.len() == b.len() && keys_match(&a, &b)
}

/// Filesystem roots: `/`, a drive root like `C:\`, or a UNC share root.
pub fn is_root_like(path: &Path) -> bool {
    let p = parts(path);
    match p.len() {
        0 => true,
        1 => {
            let key = p[0].0.as_str();
            key == "/" || (cfg!(windows) && (key.ends_with(':') || key.starts_with(r"\\")))
        }
        _ => false,
    }
}

/// `~/Library/Caches/com.foo.bar` under anchor `~/Library/Caches` has depth 1.
pub fn depth_below(anchor: &Path, path: &Path) -> Option<usize> {
    let (a, c) = (parts(anchor), parts(path));
    if keys_match(&a, &c) {
        Some(c.len() - a.len())
    } else {
        None
    }
}

/// The remainder of `path` strictly below `anchor`, preserving the path's
/// own casing; `None` when the path is the anchor or outside it.
pub fn relative_below(anchor: &Path, path: &Path) -> Option<PathBuf> {
    let (a, c) = (parts(anchor), parts(path));
    if c.len() <= a.len() || !keys_match(&a, &c) {
        return None;
    }
    Some(
        c[a.len()..]
            .iter()
            .fold(PathBuf::new(), |mut acc, (_, raw)| {
                acc.push(raw);
                acc
            }),
    )
}

/// Component-wise comparison that is stable for display and de-duplication.
pub fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A path reduced to one comparable string.
///
/// This is [`same_path`] made cheap enough to index a table with. `same_path` is
/// correct but builds a `Vec` of lowercased components per call, which is fine for
/// a dozen comparisons and quadratic when a scan asks one per cached file: a
/// 15,000-file security scan spent most of ten minutes doing nothing but
/// re-comparing paths.
///
/// The two must agree, because a key that matched where `same_path` would not let
/// a cached verdict be reused for the wrong file. Both fold case unconditionally —
/// the rule [`keys_match`] already applies on every platform — and a test asserts
/// the agreement rather than trusting this wording.
pub fn canonical_key(path: &Path) -> String {
    let parts: Vec<String> = parts(path).into_iter().map(|(key, _)| key).collect();
    // `parts` keeps the root as its own component so that an absolute path cannot
    // collide with a relative one, which would let a cached verdict be reused for
    // the wrong file. Joining them verbatim reads as `//a/b`, so the root is put
    // back as a prefix instead: same uniqueness, and the cache file on disk stays
    // legible to whoever opens it.
    match parts.first().map(String::as_str) {
        Some("/") => format!("/{}", parts[1..].join("/")),
        _ => parts.join("/"),
    }
}

/// Name that survives being shown in a one-line row: deepest meaningful label.
pub fn label_for(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_key_agrees_with_same_path() {
        let cases: [(&str, &str, bool); 7] = [
            ("/a/b/c", "/a/b/c", true),
            ("/a/b/../b/c", "/a/b/c", true),
            ("/a/./b/c", "/a/b/c", true),
            ("/a/b/c", "/a/b/d", false),
            ("/a/b/c", "/a/b", false),
            ("/a/b/c", "/a/b/c/d", false),
            ("/A/b", "/a/b", true),
        ];
        for (left, right, same) in cases {
            let (a, b) = (Path::new(left), Path::new(right));
            assert_eq!(
                same_path(a, b),
                canonical_key(a) == canonical_key(b),
                "same_path and canonical_key disagree about {left} and {right}"
            );
            assert_eq!(
                same,
                same_path(a, b),
                "unexpected same_path for {left} and {right}"
            );
        }
    }

    #[test]
    fn every_spelling_of_one_file_gives_one_key() {
        // A cache keyed by this must not answer "never seen it" to a file it has
        // already judged, or every scan re-reads the disk and the cache is a lie.
        for spelling in ["/a/b/c", "/a/b/c/", "/a/./b/c", "/a/b/../b/c"] {
            assert_eq!(
                canonical_key(Path::new(spelling)),
                canonical_key(Path::new("/a/b/c")),
                "{spelling} is the same file as /a/b/c"
            );
        }
    }

    #[test]
    fn a_canonical_key_is_usable_as_a_hash_key() {
        // Two spellings of one file must land on one slot, or a cache lookup
        // would answer "never seen it" and re-hash the whole disk every scan.
        assert_eq!(
            canonical_key(Path::new("/Users/Tester/Library/Caches")),
            canonical_key(Path::new("/Users/Tester/./Library/Caches"))
        );
        assert_ne!(
            canonical_key(Path::new("/a/b")),
            canonical_key(Path::new("/a/b/c"))
        );
    }

    #[test]
    fn expands_home() {
        let home = Path::new("/Users/tester");
        assert_eq!(
            expand_with("~/Library/Caches", home),
            PathBuf::from("/Users/tester/Library/Caches")
        );
        assert_eq!(
            expand_with("$HOME/Library", home),
            PathBuf::from("/Users/tester/Library")
        );
        assert_eq!(expand_with("~", home), home);
        assert_eq!(expand_with("/tmp", home), PathBuf::from("/tmp"));
    }

    #[test]
    fn within_and_depth() {
        let anchor = Path::new("/a/b");
        assert!(is_within(anchor, Path::new("/a/b/c")));
        assert!(!is_within(anchor, Path::new("/a/b")));
        assert_eq!(depth_below(anchor, Path::new("/a/b/c/d")), Some(2));
        assert_eq!(depth_below(anchor, Path::new("/x/y")), None);
    }

    #[test]
    fn roots_are_detected() {
        assert!(is_root_like(Path::new("/")));
        assert!(is_root_like(Path::new("")));
        assert!(!is_root_like(Path::new("/a")));
        assert!(same_path(Path::new("/a/b"), Path::new("/a/b")));
        assert!(!same_path(Path::new("/a/b"), Path::new("/a/b/c")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_compare_like_the_filesystem() {
        assert!(same_path(
            Path::new(r"C:\Users\Tester\AppData"),
            Path::new(r"c:\users\tester\appdata")
        ));
        // `canonicalize` hands back verbatim spellings with real casing.
        assert!(same_path(
            Path::new(r"\\?\C:\Users\Tester"),
            Path::new(r"c:\users\tester")
        ));
        // Drive root in both spellings is the volume root.
        assert!(is_root_like(Path::new(r"C:\")));
        assert!(is_root_like(Path::new(r"C:")));
        assert!(is_root_like(Path::new(r"\\nas\share")));
        assert!(!is_root_like(Path::new(r"C:\Users")));
        assert!(is_within(
            Path::new(r"C:\Users\Tester\AppData\Local"),
            Path::new(r"C:\users\tester\appdata\local\Temp\x")
        ));
        assert_eq!(
            relative_below(
                Path::new(r"C:\Users\Tester\AppData\Local"),
                Path::new(r"C:\users\tester\appdata\local\Temp\x")
            ),
            Some(PathBuf::from(r"Temp\x"))
        );
    }
}
