//! Filesystem primitives: fast size measurement, safe removal, Trash support.
//!
//! Sizes are measured twice over: `logical` (the `st_size` sum the Finder
//! shows) and `allocated` (`st_blocks`, what the volume actually gave up).
//! squid reports `allocated` whenever it can because that is the number that
//! matches `df`.

use crate::bytes::Bytes;
use crate::error::{Result, SquidError};
use crate::model::{DeleteMode, ItemKind};
use crate::paths;
use crate::progress::CancelToken;
use crate::safety::SafetyPolicy;
use rayon::prelude::*;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeInfo {
    pub logical: Bytes,
    pub allocated: Bytes,
    pub files: u64,
    pub dirs: u64,
    pub unreadable: u32,
}

impl SizeInfo {
    /// Bytes really occupied; see [`crate::CleanupItem::on_disk`] for why the
    /// two numbers must never be compared with `max()`.
    pub fn on_disk(&self) -> Bytes {
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

    pub fn merge(&mut self, other: &SizeInfo) {
        self.logical += other.logical;
        self.allocated += other.allocated;
        self.files += other.files;
        self.dirs += other.dirs;
        self.unreadable += other.unreadable;
    }

    /// "12,431 files" style annotation for detail rows.
    pub fn note(&self) -> String {
        let files = self.files;
        let grouped = format_thousands(files);
        if self.dirs > 0 {
            format!("{grouped} files, {} dirs", self.dirs)
        } else {
            format!("{grouped} files")
        }
    }
}

pub use crate::bytes::format_thousands;

fn allocated_bytes(meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

/// Measure one path. `Contents` skips the directory entry itself so the size
/// matches what removing its children would free.
pub fn measure(path: &Path, kind: ItemKind) -> SizeInfo {
    let mut info = SizeInfo::default();
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return info,
    };
    if meta.file_type().is_symlink() {
        info.files += 1;
        info.logical += Bytes(meta.len());
        return info;
    }
    match kind {
        ItemKind::File => {
            info.files += 1;
            info.logical += Bytes(meta.len());
            info.allocated += Bytes(allocated_bytes(&meta));
            return info;
        }
        ItemKind::Path | ItemKind::Contents => {}
    }
    if !meta.is_dir() {
        info.files += 1;
        info.logical += Bytes(meta.len());
        info.allocated += Bytes(allocated_bytes(&meta));
        return info;
    }

    let min_depth = if kind == ItemKind::Contents { 1 } else { 0 };
    let walker = WalkDir::new(path)
        .follow_links(false)
        .min_depth(min_depth)
        .max_open(64)
        .contents_first(false);

    for entry in walker.into_iter() {
        match entry {
            Ok(entry) => {
                let ft = entry.file_type();
                if ft.is_symlink() {
                    continue;
                }
                if ft.is_dir() {
                    // `Contents` starts at depth 1, so it never yields the root
                    // entry; `Path` yields it and counts it — the folder itself
                    // is part of what gets removed.
                    info.dirs += 1;
                    continue;
                }
                match entry.metadata() {
                    Ok(meta) => {
                        info.files += 1;
                        info.logical += Bytes(meta.len());
                        info.allocated += Bytes(allocated_bytes(&meta));
                    }
                    Err(_) => info.unreadable += 1,
                }
            }
            Err(_) => info.unreadable += 1,
        }
    }
    info
}

/// Measure many paths in parallel. `progress` receives completed units.
/// `Path` for folders (and anything that should be removed recursively),
/// `File` for plain files and symlinks.
pub fn kind_of(path: &Path) -> crate::model::ItemKind {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.is_dir() {
                crate::model::ItemKind::Path
            } else {
                crate::model::ItemKind::File
            }
        }
        Err(_) => crate::model::ItemKind::Path,
    }
}

pub fn measure_all(
    targets: &[(PathBuf, ItemKind)],
    cancel: &CancelToken,
    on_unit: impl Fn(usize, &SizeInfo) + Send + Sync,
) -> Vec<SizeInfo> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(measure_threads())
        .build();
    let pool = pool.unwrap_or_else(|_| {
        // Inherited pool if a nested build fails; never fail a scan for this.
        rayon::ThreadPoolBuilder::new().build().expect("rayon pool")
    });

    let done = AtomicU64::new(0);
    let cancelled = AtomicBool::new(false);
    let emit = &on_unit;

    let results: Vec<SizeInfo> = pool.install(|| {
        targets
            .par_iter()
            .map(|(path, kind)| {
                if cancel.is_cancelled() {
                    cancelled.store(true, Ordering::Relaxed);
                    return SizeInfo::default();
                }
                let info = measure(path, *kind);
                let idx = done.fetch_add(1, Ordering::Relaxed) as usize;
                emit(idx, &info);
                info
            })
            .collect()
    });
    results
}

fn measure_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    cores.clamp(4, 16)
}

/// Validate + remove one item. Returns the bytes actually released.
pub fn remove_item(item: &crate::model::CleanupItem, mode: DeleteMode) -> Result<Bytes> {
    let policy = SafetyPolicy::default();
    remove_item_with(item, mode, &policy)
}

pub fn remove_item_with(
    item: &crate::model::CleanupItem,
    mode: DeleteMode,
    policy: &SafetyPolicy,
) -> Result<Bytes> {
    let anchors = if item.anchors.is_empty() {
        default_anchors()
    } else {
        item.anchors.clone()
    };
    policy.check(&item.path, &anchors)?;

    let before = measure(&item.path, item.kind);
    match mode {
        DeleteMode::Trash => {
            trash_path(&item.path)?;
        }
        DeleteMode::Permanent => match item.kind {
            ItemKind::File => {
                std::fs::remove_file(&item.path).map_err(|e| SquidError::io(&item.path, e))?
            }
            ItemKind::Path => {
                std::fs::remove_dir_all(&item.path).map_err(|e| SquidError::io(&item.path, e))?
            }
            ItemKind::Contents => remove_contents(&item.path)?,
        },
    }

    let freed = SizeInfo {
        logical: Bytes(
            before
                .logical
                .0
                .saturating_sub(after_size(&item.path, item.kind).logical.0),
        ),
        allocated: Bytes(
            before
                .allocated
                .0
                .saturating_sub(after_size(&item.path, item.kind).allocated.0),
        ),
        files: 0,
        dirs: 0,
        unreadable: 0,
    };
    Ok(freed.on_disk().max(Bytes::ZERO))
}

fn after_size(path: &Path, kind: ItemKind) -> SizeInfo {
    if !path.exists() {
        SizeInfo::default()
    } else {
        measure(path, kind)
    }
}

fn remove_contents(dir: &Path) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| SquidError::io(dir, e))?;
    let mut first_error: Option<SquidError> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let is_link = meta
            .as_ref()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        let result = if is_dir && !is_link {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(err) = result {
            if err.kind() != io::ErrorKind::NotFound && first_error.is_none() {
                first_error = Some(SquidError::io(path, err));
            }
        }
    }
    match first_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Roots a cleanup target may legitimately live under when no rule anchors are
/// attached (interactive deletion from the TUI, history restore, …).
pub fn default_anchors() -> Vec<PathBuf> {
    let home = paths::home_dir();
    let mut anchors = vec![
        home.join("Library"),
        home.join(".cache"),
        home.join(".npm"),
        home.join(".cargo"),
        home.join(".Trash"),
        PathBuf::from("/Library/Caches"),
        PathBuf::from("/Library/Logs"),
        PathBuf::from("/Library/Application Support/CrashReporter"),
        PathBuf::from("/private/var/log"),
        PathBuf::from("/private/var/folders"),
        PathBuf::from("/Volumes"),
    ];
    anchors.push(home.join("Documents"));
    anchors.push(home.join("Downloads"));
    anchors.push(home.join("Desktop"));
    anchors.retain(|p| p.exists());
    anchors
}

/// Where `path` would land in the Trash, honouring per-volume `.Trashes`.
pub fn trash_target_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".into());
    let dir = trash_dir_for(path);
    let mut candidate = dir.join(&name);
    let mut counter = 2usize;
    while candidate.exists() {
        let (stem, ext) = split_name(&name);
        let renamed = if ext.is_empty() {
            format!("{stem} {counter}")
        } else {
            format!("{stem} {counter}.{ext}")
        };
        candidate = dir.join(renamed);
        counter += 1;
    }
    candidate
}

fn split_name(name: &str) -> (String, String) {
    // Only the last extension of bundles/apps is cosmetic; ".app" stays whole.
    if let Some(rest) = name.strip_suffix(".app") {
        return (rest.to_string(), "app".to_string());
    }
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
            (stem.to_string(), ext.to_string())
        }
        _ => (name.to_string(), String::new()),
    }
}

fn trash_dir_for(path: &Path) -> PathBuf {
    let uid = unsafe { libc::getuid() };
    let volume_root = volume_root_for(path);
    if !volume_root.as_os_str().is_empty() {
        let per_volume = volume_root.join(".Trashes").join(uid.to_string());
        if can_write(&per_volume) || ensure_dir(&per_volume) {
            return per_volume;
        }
    }
    let home_trash = paths::home_dir().join(".Trash");
    if ensure_dir(&home_trash) {
        return home_trash;
    }
    paths::home_dir().join(".Trash")
}

fn can_write(dir: &Path) -> bool {
    let probe = dir.join(format!(".squid-probe-{}", std::process::id()));
    match OpenOptions::new().create_new(true).write(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn ensure_dir(dir: &Path) -> bool {
    if dir.is_dir() {
        return true;
    }
    std::fs::create_dir_all(dir).is_ok()
}

pub fn volume_root_for(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let s = abs.to_string_lossy().into_owned();
    if let Some(rest) = s.strip_prefix("/Volumes/") {
        if let Some(idx) = rest.find('/') {
            return PathBuf::from(format!("/Volumes/{}", &rest[..idx]));
        }
        return PathBuf::from(s);
    }
    if s.starts_with("/System/Volumes/Data") {
        return PathBuf::from("/System/Volumes/Data");
    }
    PathBuf::from("/")
}

/// Move to Trash, copying first when the target lives on another volume.
pub fn trash_path(path: &Path) -> Result<PathBuf> {
    let destination = trash_target_for(path);
    match std::fs::rename(path, &destination) {
        Ok(()) => Ok(destination),
        Err(_) => {
            copy_tree(path, &destination)?;
            let kind = if path.is_dir() {
                ItemKind::Path
            } else {
                ItemKind::File
            };
            match kind {
                ItemKind::Path => {
                    let _ = std::fs::remove_dir_all(path);
                }
                ItemKind::File | ItemKind::Contents => {
                    std::fs::remove_file(path).map_err(|e| SquidError::io(path, e))?;
                }
            }
            Ok(destination)
        }
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(from).map_err(|e| SquidError::io(from, e))?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(from).map_err(|e| SquidError::io(from, e))?;
        std::os::unix::fs::symlink(&target, to).map_err(|e| SquidError::io(to, e))?;
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to).map_err(|e| SquidError::io(to, e))?;
        let entries = std::fs::read_dir(from).map_err(|e| SquidError::io(from, e))?;
        for entry in entries.flatten() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }
    let mut reader = File::open(from).map_err(|e| SquidError::io(from, e))?;
    let mut writer = File::create(to).map_err(|e| SquidError::io(to, e))?;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| SquidError::io(from, e))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|e| SquidError::io(to, e))?;
    }
    Ok(())
}

/// Empty the user's Trash. Deliberately shallow: only top level entries.
pub fn empty_trash() -> Result<(usize, Bytes)> {
    let trash = paths::home_dir().join(".Trash");
    if !trash.is_dir() {
        return Ok((0, Bytes::ZERO));
    }
    let mut removed = 0usize;
    let mut freed = Bytes::ZERO;
    let entries: Vec<PathBuf> = std::fs::read_dir(&trash)
        .map_err(|e| SquidError::io(&trash, e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .map(|n| n.to_string_lossy().starts_with(".squid-probe-"))
                .unwrap_or(false)
        })
        .collect();
    for path in entries {
        let size = measure(&path, ItemKind::Path);
        let allocated = size.on_disk();
        let result = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if result.is_ok() {
            removed += 1;
            freed += allocated;
        }
    }
    Ok((removed, freed))
}

/// Shared handle type used by long-running jobs.
pub type SharedCancel = Arc<CancelToken>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn measures_contents_and_path() {
        let root = std::env::temp_dir().join(format!("squid-size-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/file.bin"), vec![7u8; 4096]).unwrap();

        let whole = measure(&root, ItemKind::Path);
        assert!(whole.logical.as_u64() >= 4096);
        assert_eq!(whole.files, 1);

        // `Contents` still counts the children; the directory entry has no size
        // of its own, so totals match but the top dir is not counted.
        let contents = measure(&root, ItemKind::Contents);
        assert_eq!(contents.files, 1);
        assert!(contents.dirs < whole.dirs);
        let _ = fs::remove_dir_all(&root);
    }

    /// A thin-provisioned disk image is the reason `max(logical, allocated)`
    /// is forbidden: Docker's `Docker.raw` reports a terabyte of logical
    /// length while the SSD behind it holds 256 GB in total.
    #[test]
    fn a_sparse_file_counts_for_what_it_really_uses() {
        let root = std::env::temp_dir().join(format!("squid-sparse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let raw = root.join("Docker.raw");
        let file = fs::File::create(&raw).unwrap();
        file.set_len(8 << 30).unwrap();
        drop(file);

        let info = measure(&root, ItemKind::Path);
        assert!(info.logical.as_u64() >= 8 << 30, "logical length is 8 GiB");
        assert!(
            info.on_disk().as_u64() * 100 < info.logical.as_u64(),
            "the walk reported {}/{}; a sparse file must count as the blocks              it wrote",
            info.on_disk(),
            info.logical
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn thousand_separators() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(1234), "1,234");
        assert_eq!(format_thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn trash_target_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("squid-trash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim");
        fs::write(&victim, b"hi").unwrap();
        let first = trash_target_for(&victim);
        assert!(first.starts_with(trash_dir_for(&victim)));
        let _ = fs::remove_dir_all(&dir);
    }
}
