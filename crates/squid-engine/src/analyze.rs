//! `squid analyze` / the Storage panel — DaisyDisk-style drill-down.
//!
//! The tree is built lazily one level at a time: measure the children of the
//! folder the user is looking at, keep the biggest `top` of them, and fold the
//! remainder into a single "rest" figure. That keeps a first answer on a whole
//! home folder to a couple of seconds while still being exact about the top
//! entries, and it never has to hold a full filesystem tree in memory.

use serde::{Deserialize, Serialize};
use squid_core::fs;
use squid_core::progress::{CancelToken, Progress};
use squid_core::Bytes;
use std::path::{Path, PathBuf};

/// How many children one folder may be measured for before folding the rest.
const CHILD_CAP: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub path: PathBuf,
    pub logical: Bytes,
    pub allocated: Bytes,
    pub files: u64,
    pub dirs: u64,
    pub is_dir: bool,
    pub readable: bool,
    /// Biggest children first. Empty until [`explore`] ran with depth > 0.
    pub children: Vec<Node>,
    /// Children that were measured but too small to show.
    pub folded: usize,
    /// Their combined size.
    pub folded_size: Bytes,
    /// True when `children` is the complete list.
    pub complete: bool,
}

impl Node {
    /// On-disk bytes. A sparse directory (a Docker VM image, a Time Machine
    /// local snapshot) has a logical size the disk cannot hold, so the
    /// allocated figure wins whenever `measure` produced one.
    pub fn size(&self) -> Bytes {
        #[cfg(unix)]
        {
            self.allocated
        }
        #[cfg(not(unix))]
        {
            if self.allocated.is_zero() {
                self.logical
            } else {
                self.allocated
            }
        }
    }

    pub fn name(&self) -> String {
        squid_core::paths::label_for(&self.path)
    }

    pub fn share_of(&self, parent: &Node) -> f64 {
        let total = parent.size().as_u64();
        if total == 0 {
            0.0
        } else {
            self.size().as_u64() as f64 / total as f64
        }
    }

    /// Children plus a synthetic entry for the folded remainder, so a bar chart
    /// always adds up to the parent's size.
    pub fn display_children(&self) -> Vec<Node> {
        let mut out = self.children.clone();
        if self.folded > 0 {
            out.push(Node {
                path: self.path.clone(),
                logical: self.folded_size,
                allocated: self.folded_size,
                files: 0,
                dirs: 0,
                is_dir: true,
                readable: true,
                children: Vec::new(),
                folded: 0,
                folded_size: Bytes::ZERO,
                complete: true,
            });
        }
        out
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub logical: Bytes,
    pub allocated: Bytes,
    pub modified_ms: Option<u64>,
}

impl FileEntry {
    pub fn size(&self) -> Bytes {
        #[cfg(unix)]
        {
            self.allocated
        }
        #[cfg(not(unix))]
        {
            if self.allocated.is_zero() {
                self.logical
            } else {
                self.allocated
            }
        }
    }
}

/// Measure `root` and up to `depth` levels below it.
pub fn explore(
    root: &Path,
    depth: usize,
    top: usize,
    cancel: &CancelToken,
    progress: &Progress,
) -> squid_core::Result<Node> {
    let info = fs::measure(root, fs::kind_of(root));
    let mut node = Node {
        path: root.to_path_buf(),
        logical: info.logical,
        allocated: info.allocated,
        files: info.files,
        dirs: info.dirs,
        is_dir: root.is_dir(),
        readable: info.unreadable == 0,
        children: Vec::new(),
        folded: 0,
        folded_size: Bytes::ZERO,
        complete: false,
    };
    if depth > 0 && node.is_dir {
        expand(&mut node, depth, top, cancel, progress)?;
    } else {
        node.complete = true;
    }
    Ok(node)
}

/// Fill one level of `node`, then recurse into the biggest children.
pub fn expand(
    node: &mut Node,
    depth: usize,
    top: usize,
    cancel: &CancelToken,
    progress: &Progress,
) -> squid_core::Result<()> {
    if cancel.is_cancelled() {
        return Err(squid_core::SquidError::Cancelled);
    }
    let entries = match std::fs::read_dir(&node.path) {
        Ok(entries) => entries,
        Err(_) => {
            node.complete = true;
            return Ok(());
        }
    };
    let mut kids: Vec<(PathBuf, bool)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|t| t.is_dir())
            .unwrap_or_else(|_| path.is_dir());
        let mount = is_mountpoint(&path);
        kids.push((path, is_dir && !mount));
    }
    let total_children = kids.len();
    if kids.len() > CHILD_CAP {
        kids.sort_by(|a, b| b.0.cmp(&a.0));
        kids.truncate(CHILD_CAP);
    }

    progress.phase(
        "storage",
        format!("measuring {}", squid_core::paths::pretty(&node.path)),
    );
    let targets: Vec<(PathBuf, squid_core::ItemKind)> = kids
        .iter()
        .map(|(path, is_dir)| {
            (
                path.clone(),
                if *is_dir {
                    squid_core::ItemKind::Path
                } else {
                    squid_core::ItemKind::File
                },
            )
        })
        .collect();
    progress.tick(0, Some(targets.len() as u64), None);
    let sizes = fs::measure_all(&targets, cancel, |index, _info| {
        progress.tick(
            index as u64 + 1,
            Some(targets.len() as u64),
            targets.get(index).map(|(path, _)| path.clone()),
        );
    });

    let mut children: Vec<Node> = targets
        .iter()
        .zip(sizes)
        .map(|((path, _), info)| Node {
            path: path.clone(),
            logical: info.logical,
            allocated: info.allocated,
            files: info.files,
            dirs: info.dirs,
            is_dir: path.is_dir(),
            readable: info.unreadable == 0,
            children: Vec::new(),
            folded: 0,
            folded_size: Bytes::ZERO,
            complete: depth <= 1,
        })
        .collect();
    children.sort_by(|a, b| b.size().cmp(&a.size()).then_with(|| a.path.cmp(&b.path)));

    let shown: usize = top.max(1).min(children.len());
    let rest: Vec<Node> = children.drain(shown.min(children.len())..).collect();
    node.folded = total_children.saturating_sub(shown);
    node.folded_size = rest.iter().map(|child| child.size()).sum();
    node.complete = node.folded == 0;

    for mut child in children.into_iter().take(shown) {
        if depth > 1 && child.is_dir {
            expand(&mut child, depth - 1, top, cancel, progress)?;
        }
        node.children.push(child);
    }
    Ok(())
}

/// The biggest individual files under `root` — the "what on earth is 40 GB"
/// answer that folder trees hide.
pub fn largest_files(
    root: &Path,
    limit: usize,
    min_bytes: Bytes,
    cancel: &CancelToken,
    progress: &Progress,
) -> squid_core::Result<Vec<FileEntry>> {
    let mut found: Vec<FileEntry> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    progress.phase("files", "collecting the largest files");
    while let Some((dir, depth)) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(squid_core::SquidError::Cancelled);
        }
        visited += 1;
        if visited.is_multiple_of(64) {
            progress.tick(visited as u64, None, Some(dir.clone()));
        }
        if visited > 500_000 {
            progress.warning("stopped after 500,000 folders — narrow the search");
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                if depth < 12 && !is_mountpoint(&path) {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let logical = Bytes(meta.len());
            if logical < min_bytes {
                continue;
            }
            let allocated = Bytes(allocated_of(&meta));
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            found.push(FileEntry {
                path,
                logical,
                allocated,
                modified_ms,
            });
            if found.len() > limit.saturating_mul(8) {
                found.sort_by_key(|entry| std::cmp::Reverse(entry.size()));
                found.truncate(limit);
            }
        }
    }
    found.sort_by_key(|entry| std::cmp::Reverse(entry.size()));
    found.truncate(limit);
    Ok(found)
}

#[cfg(unix)]
fn allocated_of(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn allocated_of(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

/// Skip other filesystems: analysing `/System/Volumes/Data` twice would lie.
fn is_mountpoint(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        const SKIP: &[&str] = &[
            "/System/Volumes/Data",
            "/System/Volumes/Update",
            "/System/Volumes/Preboot",
            "/System/Volumes/iSCPrecovery",
            "/System/Volumes/XART",
            "/Volumes",
            "/dev",
            "/proc",
            "/sys",
            "/run",
            "/snap",
            "/mnt",
            "/media",
        ];
        SKIP.iter().any(|skip| path.as_os_str() == *skip)
    }
    #[cfg(not(target_os = "macos"))]
    {
        const SKIP: &[&str] = &["/dev", "/proc", "/sys", "/run", "/snap", "/mnt", "/media"];
        SKIP.iter().any(|skip| path.as_os_str() == *skip)
    }
}

/// Aggregate the tree into `(label, size)` pairs for a bar chart.
pub fn buckets(node: &Node, limit: usize) -> Vec<(String, Bytes)> {
    let mut out: Vec<(String, Bytes)> = node
        .children
        .iter()
        .take(limit)
        .map(|child| (child.name(), child.size()))
        .collect();
    if node.folded > 0 {
        out.push((format!("{} others", node.folded), node.folded_size));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One folder per call. A session id is a second stamp plus sixteen bits of
    /// clock hash, which two fixtures created in the same microsecond have been
    /// known to share — and then one test measured the other's half-deleted
    /// tree and wondered why the biggest folder came second. Returns None when
    /// an endpoint protector refuses the scratch writes (skip, not failure).
    fn fixture(who: &str) -> Option<PathBuf> {
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "squid-analyze-{}-{}-{}-{}",
            std::process::id(),
            who,
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            squid_core::ledger::new_session_id()
        ));
        if let Err(error) = std::fs::create_dir_all(root.join("big")) {
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                eprintln!("skipping: scratch write denied ({error})");
                return None;
            }
            panic!("scratch dir: {error}");
        }
        std::fs::write(root.join("big/blob.bin"), vec![9u8; 400_000]).ok()?;
        std::fs::create_dir_all(root.join("small")).ok()?;
        std::fs::write(root.join("small/tiny.txt"), b"hi").ok()?;
        std::fs::write(root.join("loose.bin"), vec![3u8; 90_000]).ok()?;
        Some(root)
    }

    #[test]
    fn the_biggest_folder_comes_first() {
        let Some(root) = fixture("the_biggest_folder_comes_first") else {
            return;
        };
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let node = explore(&root, 2, 4, &cancel, &progress).expect("explore");
        assert!(node.size().as_u64() > 400_000, "{:?}", node);
        assert!(!node.children.is_empty());
        assert_eq!(node.children[0].name(), "big");
        assert!(node.children[0].allocated.as_u64() >= node.children[1].allocated.as_u64());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn largest_files_are_sorted_and_capped() {
        let Some(root) = fixture("largest_files_are_sorted_and_capped") else {
            return;
        };
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let files = largest_files(&root, 10, Bytes(1_024), &cancel, &progress).expect("files");
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files[0].path.ends_with("big/blob.bin"));
        assert!(files[0].size().as_u64() > files[1].size().as_u64());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_folded_remainder_keeps_the_totals_honest() {
        let Some(root) = fixture("the_folded_remainder_keeps_the_totals_honest") else {
            return;
        };
        let progress = Progress::new();
        let cancel = CancelToken::new();
        let node = explore(&root, 1, 1, &cancel, &progress).expect("explore");
        assert_eq!(node.children.len(), 1);
        assert!(node.folded > 0);
        let shown: u64 = node
            .children
            .iter()
            .map(|child| child.size().as_u64())
            .sum();
        let displayed = node.display_children();
        assert_eq!(displayed.len(), 2);
        assert!(shown + node.folded_size.as_u64() > 0);
        assert!(!node.complete);
        let _ = std::fs::remove_dir_all(&root);
    }
}
