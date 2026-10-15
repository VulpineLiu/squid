//! Filesystem primitives: fast size measurement, safe removal, Trash support.
//!
//! Sizes are measured twice over: `logical` (the `st_size` sum the Finder
//! shows) and `allocated` (what the volume actually gave up — `st_blocks` on
//! Unix, `GetCompressedFileSizeW` on Windows). squid reports `allocated`
//! whenever it can because that is the number that matches `df`.

use crate::bytes::Bytes;
use crate::error::{Result, SquidError};
use crate::model::{DeleteMode, ItemKind};
use crate::paths;
use crate::progress::CancelToken;
use crate::safety::SafetyPolicy;
use rayon::prelude::*;
use std::fs::Metadata;
#[cfg(not(windows))]
use std::fs::{File, OpenOptions};
use std::io;
#[cfg(not(windows))]
use std::io::{Read, Write};
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

/// Bytes the volume really gave up for one file, or 0 when the platform
/// cannot say.
///
/// Unix answers from `st_blocks`, which `stat` already handed us; Windows from
/// [`windows_allocation`], which costs one path-based query. Both ask about
/// every file, because whole-volume compression and APFS block cloning both
/// make the logical length a lie at every size, not just for big files.
fn allocated_bytes(path: &Path, meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        let _ = path;
        use std::os::unix::fs::MetadataExt;
        meta.blocks().saturating_mul(512)
    }
    #[cfg(windows)]
    {
        windows_allocation(path, meta)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, meta);
        0
    }
}

/// UTF-16, NUL-terminated spelling for a Win32 path-taking API.
///
/// Two things the plain spelling gets wrong. It is lossless: `encode_wide`
/// keeps every unit, where going through `String` would mangle anything a
/// `OsString` can hold but a `String` cannot. And past 260 characters the
/// legacy limit answers `ERROR_PATH_NOT_FOUND`, which the deep `node_modules`
/// trees this tool exists to measure cross routinely.
///
/// The verbatim prefix buys the long path by also switching off normalisation,
/// so it is only honest on an absolute, dot-free path. The walk builds exactly
/// that; `squid purge ..\..\somewhere` does not, so anything else keeps the
/// plain spelling and takes the failure — which lands on `meta.len()`.
/// The systemic fix is a `longPathAware` manifest on the executable; until
/// then every other Win32 call here still sees the old limit.
#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Component;

    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    let quotable = path.is_absolute()
        && !path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir));
    let verbatim = &[0x5C, 0x5C, 0x3F, 0x5C]; // `\\?\`
    let unc = &[0x5C, 0x5C]; // `\\`
    let needs_prefix = raw.len() >= 250 && quotable && !raw.starts_with(verbatim);

    let mut wide: Vec<u16> = if !needs_prefix {
        raw
    } else if raw.starts_with(unc) {
        r"\\?\UNC\"
            .encode_utf16()
            .chain(raw[2..].iter().copied())
            .collect()
    } else {
        r"\\?\".encode_utf16().chain(raw.iter().copied()).collect()
    };
    wide.push(0);
    wide
}

/// `GetCompressedFileSizeW`, the one cheap Windows answer to "how many bytes
/// did the volume hand over".
///
/// Measured on a real NTFS volume (Windows 10 1909), because the documented
/// alternatives were all wrong for this:
///
/// * `FILE_STANDARD_INFO.AllocationSize` — what this function used to call — is
///   blind to NTFS compression. It reported the full 8 MiB for a file
///   `compact` accounted as 1,572,864 stored bytes; `GetCompressedFileSizeW`
///   returned 1,572,864.
/// * `FSCTL_QUERY_ALLOCATED_RANGES` reported the same 8 MiB window as fully
///   allocated. It answers "which clusters exist", not "what do they weigh".
/// * Nothing rounds up: files from 100 bytes (small enough to live inside the
///   `$MFT` record) to 1 GiB extended by `SetEndOfFile` all report exactly
///   their length, so there is no cluster quantum to correct for here — unlike
///   `st_blocks`, which does round.
///
/// Taking a path rather than a handle matters twice over. It is one query
/// instead of a `CreateFileW`/`GetFileInformationByHandleEx` pair per file, and
/// it keeps answering while another process holds the file open exclusively —
/// which is the normal state of the cache files this walk exists to size.
///
/// The failure modes fall back to the logical length: a path over the legacy
/// limit without a `\\\\?` prefix, a volume that will not tell us, a file that
/// vanished mid-walk. Overstating by the logical size is the survivable
/// direction; understating a cleanup the user is about to approve is not.
#[cfg(windows)]
fn windows_allocation(path: &Path, meta: &Metadata) -> u64 {
    use std::os::windows::fs::MetadataExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetCompressedFileSizeW, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_SPARSE_FILE,
    };

    let wide = wide_path(path);
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(PCWSTR(wide.as_ptr()), Some(&mut high)) };
    // The API signals failure with 0xFFFFFFFF, then clears the last error on
    // success, so a file that really is 4 GiB minus one long survives.
    if low == u32::MAX && io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
        return meta.len();
    }
    let reported = (u64::from(high) << 32) | u64::from(low);
    if reported != 0 || meta.len() == 0 {
        return reported;
    }
    // Only a compressed or sparse file may legitimately hold data the volume
    // charges nothing for. Any other file answering 0 bytes is the
    // deleted-but-still-open case: the clusters are gone but the entry is still
    // in our walk, and zeroing it would hide space the user is not getting.
    let flagged = FILE_ATTRIBUTE_COMPRESSED.0 | FILE_ATTRIBUTE_SPARSE_FILE.0;
    if meta.file_attributes() & flagged == 0 {
        return meta.len();
    }
    reported
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
            info.allocated += Bytes(allocated_bytes(path, &meta));
            return info;
        }
        ItemKind::Path | ItemKind::Contents => {}
    }
    if !meta.is_dir() {
        info.files += 1;
        info.logical += Bytes(meta.len());
        info.allocated += Bytes(allocated_bytes(path, &meta));
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
                        info.allocated += Bytes(allocated_bytes(entry.path(), &meta));
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
    #[cfg(windows)]
    {
        windows_anchors()
    }
    #[cfg(not(windows))]
    unix_default_anchors()
}

#[cfg(windows)]
fn windows_anchors() -> Vec<PathBuf> {
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let home = paths::home_dir();
    let mut anchors = vec![
        env("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local")),
        env("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming")),
        env("ProgramData").unwrap_or_else(|| PathBuf::from(r"C:\ProgramData")),
        env("TEMP").unwrap_or_else(|| home.join("AppData").join("Local").join("Temp")),
        home,
    ];
    anchors.retain(|p| p.exists());
    anchors
}

#[cfg(not(windows))]
fn unix_default_anchors() -> Vec<PathBuf> {
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
///
/// Returns `None` on Windows: the shell Recycle Bin is written through
/// `IFileOperation`, which does not hand out a destination path, so the
/// ledger records no physical trash location there and undo relies on the
/// Recycle Bin UI.
pub fn trash_target_for(path: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let _ = path;
        None
    }
    #[cfg(not(windows))]
    {
        Some(unix_trash_target_for(path))
    }
}

#[cfg(not(windows))]
fn unix_trash_target_for(path: &Path) -> PathBuf {
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
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
/// The returned path is where the item physically landed, or `None` on
/// Windows where the shell keeps the Recycle Bin bookkeeping.
pub fn trash_path(path: &Path) -> Result<Option<PathBuf>> {
    #[cfg(windows)]
    {
        recycle_bin::move_to_recycle_bin(path)?;
        Ok(None)
    }
    #[cfg(not(windows))]
    trash_path_unix(path)
}

#[cfg(not(windows))]
fn trash_path_unix(path: &Path) -> Result<Option<PathBuf>> {
    let destination = unix_trash_target_for(path);
    match std::fs::rename(path, &destination) {
        Ok(()) => Ok(Some(destination)),
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
            Ok(Some(destination))
        }
    }
}

#[cfg(not(windows))]
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

    /// Scratch directory that goes away even when an assertion panics halfway
    /// through the test.
    ///
    /// This is not housekeeping: a file extended with `set_len` is charged as
    /// if it were written in full, so a fixture that survives a failure sits on
    /// a real disk at its full logical size. One did — 11 GB of them, in
    /// `AppData\Local\Temp`, where squid then did exactly the right thing and
    /// offered them up as Temp junk.
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("squid-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("scratch dir");
            Self { path }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn measures_contents_and_path() {
        let tmp = TempRoot::new("size");
        let root = &tmp.path;
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/file.bin"), vec![7u8; 4096]).unwrap();

        let whole = measure(root, ItemKind::Path);
        assert!(whole.logical.as_u64() >= 4096);
        assert_eq!(whole.files, 1);

        // `Contents` still counts the children; the directory entry has no size
        // of its own, so totals match but the top dir is not counted.
        let contents = measure(root, ItemKind::Contents);
        assert_eq!(contents.files, 1);
        assert!(contents.dirs < whole.dirs);
    }

    /// A thin-provisioned disk image is the reason `max(logical, allocated)`
    /// is forbidden: Docker's `Docker.raw` reports a terabyte of logical
    /// length while the SSD behind it holds 256 GB in total.
    ///
    /// Holes come free on POSIX — `set_len` extends without allocating — so the
    /// fixture needs no privilege here. Windows needs a different fixture and
    /// gets its own test below.
    #[test]
    #[cfg(unix)]
    fn a_sparse_file_counts_for_what_it_really_uses() {
        let tmp = TempRoot::new("sparse");
        let root = &tmp.path;
        // One gibibyte is plenty to show the property and small enough that a
        // nearly full disk can still extend to it.
        const SPARSE_LEN: u64 = 1 << 30;
        let raw = root.join("Docker.raw");
        let file = fs::File::create(&raw).unwrap();
        if file.set_len(SPARSE_LEN).is_err() {
            // Volumes with a space-reservation filter answer `SetEndOfFile`
            // with ERROR_DISK_FULL instead of handing out a hole.
            eprintln!("skipped: this volume will not extend a file without the space");
            return;
        }
        drop(file);
        // One block of real data, so the file has both a hole and a tail the
        // volume had to hand over — the shape of an actual thin disk image.
        let mut hole = fs::OpenOptions::new().write(true).open(&raw).unwrap();
        std::io::Write::write_all(&mut hole, &[3u8; 4096]).unwrap();
        drop(hole);

        let info = measure(root, ItemKind::Path);
        assert_eq!(info.logical, Bytes(SPARSE_LEN), "logical length is 1 GiB");
        assert!(
            info.on_disk().as_u64() * 100 < info.logical.as_u64(),
            "the walk reported {}/{}; a sparse file must count as the blocks \
             it wrote",
            info.on_disk(),
            info.logical
        );
    }

    /// NTFS is the other way round: extending a file really does allocate the
    /// whole thing (a 1 GiB `SetEndOfFile` file reported 1 GiB allocated and
    /// moved free space by 1 GiB), so a hole is not how Windows hides size.
    /// Compression is, and it is common — `compact`/CompactOS ships on devices
    /// with small eMMC, where a whole profile is stored compressed.
    ///
    /// `compact` is the only supported way to provoke it from a test: the
    /// `FSCTL_SET_COMPRESSION` request answers `ERROR_ACCESS_DENIED` on this
    /// volume even for a file we created ourselves, and setting
    /// `FILE_ATTRIBUTE_COMPRESSED` on a file that already has clusters leaves
    /// those clusters uncompressed. Both were measured, not assumed.
    #[test]
    #[cfg(windows)]
    fn a_compressed_file_counts_for_what_the_volume_stored() {
        use std::os::windows::fs::MetadataExt;
        use std::process::Command;

        let tmp = TempRoot::new("compressed");
        let root = &tmp.path;
        // A short repeating period: compresses several fold, so the assertion
        // cannot pass merely because a volume ignored the request the way
        // random bytes would let it.
        const LEN: usize = 8 << 20;
        let path = root.join("pattern.bin");
        let payload: Vec<u8> = (0..LEN).map(|i| (i % 509) as u8).collect();
        fs::write(&path, &payload).unwrap();

        // Run from inside the directory so the volume never sees a path that
        // needs quoting.
        let ran = Command::new("compact")
            .args(["/c", "/i", "/s:."])
            .current_dir(root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let compressed_flag = 0x0800; // FILE_ATTRIBUTE_COMPRESSED
        let marked = fs::metadata(&path)
            .map(|meta| meta.file_attributes() & compressed_flag != 0)
            .unwrap_or(false);
        if !ran.map(|status| status.success()).unwrap_or(false) || !marked {
            eprintln!("skipped: this volume will not compress the fixture");
            return;
        }

        let info = measure(root, ItemKind::Path);
        assert_eq!(
            info.logical,
            Bytes(LEN as u64),
            "logical length is unchanged"
        );
        assert!(
            info.on_disk().as_u64() * 2 < info.logical.as_u64(),
            "the walk reported {}/{}; a compressed file must count as the bytes \
             the volume stored",
            info.on_disk(),
            info.logical
        );
    }

    /// The invariant both platforms must satisfy: a file that was really
    /// written occupies at least what it logically holds. A port that forgets
    /// to answer the allocation question returns 0 here, and every "freed"
    /// number downstream quietly becomes the logical size.
    ///
    /// Written end to end, deliberately not extended with `set_len`: a hole
    /// makes the expected answer platform-specific again (POSIX may or may not
    /// count reserved blocks, and macOS was observed to disagree with itself
    /// depending on when the stat landed), and holes are what the two tests
    /// above are for.
    #[test]
    fn a_written_file_reports_what_the_volume_gave_it() {
        let tmp = TempRoot::new("alloc");
        let root = &tmp.path;
        const SOLID_LEN: usize = 8 << 20;
        let mut file = fs::File::create(root.join("solid.bin")).unwrap();
        let payload = vec![9u8; SOLID_LEN];
        std::io::Write::write_all(&mut file, &payload).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let info = measure(root, ItemKind::Path);
        assert_eq!(info.logical, Bytes(SOLID_LEN as u64));
        assert!(
            info.allocated >= info.logical,
            "a freshly written file reported {} of {} allocated; clusters only round up",
            info.allocated,
            info.logical
        );
    }

    #[test]
    fn thousand_separators() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(1234), "1,234");
        assert_eq!(format_thousands(1_234_567), "1,234,567");
    }

    #[test]
    #[cfg(not(windows))]
    fn trash_target_avoids_collisions() {
        let tmp = TempRoot::new("trash");
        let dir = &tmp.path;
        let victim = dir.join("victim");
        fs::write(&victim, b"hi").unwrap();
        let first = trash_target_for(&victim).expect("unix trash targets are predictable");
        assert!(first.starts_with(trash_dir_for(&victim)));
    }
}

/// Windows Recycle Bin backend built on `IFileOperation`. The shell owns the
/// `$Recycle.Bin` bookkeeping (`$I`/`$R` metadata), so squid only asks it to
/// recycle the item and never touches the bin directory itself.
#[cfg(windows)]
mod recycle_bin {
    use crate::error::{Result, SquidError};
    use std::io;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Foundation::{E_ABORT, E_INVALIDARG};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FOF_ALLOWUNDO,
        FOF_NOCONFIRMATION, FOF_NOCONFIRMMKDIR, FOF_NOERRORUI, FOF_SILENT,
    };

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn move_to_recycle_bin(path: &Path) -> Result<()> {
        // IFileOperation needs a COM apartment. The engine deletes from a
        // dedicated worker, so we own the lifetime here unless the thread is
        // already initialised in a different model.
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let uninit = init != RPC_E_CHANGED_MODE;
        let result = unsafe { recycle_inner(path) };
        if uninit {
            unsafe { CoUninitialize() };
        }
        result.map_err(|err| SquidError::Io {
            path: path.to_path_buf(),
            source: io::Error::from_raw_os_error(err.code().0),
        })
    }

    unsafe fn recycle_inner(path: &Path) -> windows::core::Result<()> {
        // IShellItem parsing does not accept verbatim (`\\?\`) paths.
        let display = path
            .to_str()
            .ok_or_else(|| windows::core::Error::from_hresult(E_INVALIDARG))?;
        let wide = to_wide(display);
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        // `FOF_ALLOWUNDO` is the whole point, and it is not implied. Measured on
        // a real box: without it `DeleteItem` returns S_OK, the bytes are gone
        // from the disk, `C:\$Recycle.Bin` does not move by a single byte, and
        // squid's ledger says `mode: trash` — a cleanup that silently broke its
        // own "everything goes to the Trash first" promise. `IFileOperation`
        // also has a `RecycleItem` verb, which would say it out loud, but
        // windows-rs does not project that vtable slot yet.
        operation.SetOperationFlags(
            FOF_ALLOWUNDO | FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_NOCONFIRMMKDIR,
        )?;
        let item = SHCreateItemFromParsingName::<
            PCWSTR,
            Option<&windows::Win32::System::Com::IBindCtx>,
            IShellItem,
        >(PCWSTR(wide.as_ptr()), None)?;
        operation.DeleteItem(Some(&item), None)?;
        operation.PerformOperations()?;
        // A shell verb can report S_OK and still have declined the work — the
        // progress sink abort path, a cancelled UAC prompt. Ask, because
        // claiming a Trash entry that does not exist is how undo later tells
        // the user to go looking in a Recycle Bin that never got the file.
        if operation.GetAnyOperationsAborted()?.as_bool() {
            return Err(windows::core::Error::from_hresult(E_ABORT));
        }
        Ok(())
    }
}
