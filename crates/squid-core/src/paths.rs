//! Path expansion helpers shared by the rule engine and the UI.

use std::path::{Component, Path, PathBuf};

/// Expand a leading `~` / `$HOME` and a few well known macOS anchors.
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
    if let Some(rest) = input.strip_prefix("~/") {
        return home.join(rest);
    }
    if let Some(rest) = input.strip_prefix("$HOME/") {
        return home.join(rest);
    }
    if let Some(rest) = input.strip_prefix("$HOME") {
        return home.join(rest.trim_start_matches('/'));
    }
    PathBuf::from(input)
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Best-effort `/Volumes/...`-aware display shortening for the UI.
pub fn pretty(path: &Path) -> String {
    let home = home_dir();
    match path.strip_prefix(&home) {
        Ok(rel) if rel.as_os_str().is_empty() => "~".to_string(),
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

/// True when `child` is strictly inside `parent`.
pub fn is_within(parent: &Path, child: &Path) -> bool {
    child != parent && child.starts_with(parent)
}

/// `~/Library/Caches/com.foo.bar` under anchor `~/Library/Caches` has depth 1.
pub fn depth_below(anchor: &Path, path: &Path) -> Option<usize> {
    path.strip_prefix(anchor).ok().map(|rel| {
        rel.components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .count()
    })
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
}
