//! Volume enumeration shared by the Unix adapters.

use squid_core::Bytes;
use squid_core::Volume;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct MountEntry {
    pub mount_point: PathBuf,
    pub device: String,
    pub fstype: String,
}

/// Filesystems that never represent user-visible capacity.
const PSEUDO_FS: &[&str] = &[
    "devfs",
    "autofs",
    "devtmpfs",
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "tmpfs",
    "ramfs",
    "overlay",
    "squashfs",
    "debugfs",
    "tracefs",
    "securityfs",
    "pstore",
    "binfmt_misc",
    "configfs",
    "fusectl",
    "mqueue",
    "hugetlbfs",
    "rpc_pipefs",
    "nsfs",
    "autofs",
    "nullfs",
    "msdos",
    "anon:osd",
    "SMB",
    "afpfs",
    "WebDAV",
    "NFS",
];

/// `(total, available)` for a mount point.
pub fn statvfs_for(path: &Path) -> Option<(Bytes, Bytes)> {
    use std::ffi::CString;
    let c_path = CString::new(path.as_os_str().to_string_lossy().as_bytes()).ok()?;
    let mut stat = unsafe { std::mem::zeroed::<libc::statvfs>() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let frsize = stat.f_frsize as u64;
    let block = if stat.f_blocks == 0 {
        stat.f_bsize as u64
    } else {
        frsize
    };
    let total = stat.f_blocks as u64 * block;
    let free = stat.f_bavail as u64 * block;
    if total == 0 {
        return None;
    }
    Some((Bytes(total), Bytes(free)))
}

/// Turn mount entries into volumes, skipping pseudo filesystems and deduping
/// firmlink/bind mounts that report identical capacity (macOS shows `/` and
/// `/System/Volumes/Data` as the same store).
pub fn volumes_from_mounts(mounts: &[MountEntry]) -> Vec<Volume> {
    let root = PathBuf::from("/");
    let mut seen_capacity: Vec<(u64, u64)> = Vec::new();
    let mut out: Vec<Volume> = Vec::new();
    let mut mounts: Vec<&MountEntry> = mounts.iter().collect();
    mounts.sort_by_key(|m| (m.mount_point != root, m.mount_point.clone()));

    for mount in mounts {
        let fstype_lower = mount.fstype.to_lowercase();
        if PSEUDO_FS.iter().any(|p| fstype_lower == *p) {
            continue;
        }
        let text = mount.mount_point.to_string_lossy().into_owned();
        // macOS firmlinks and snapshot mounts.
        if text.starts_with("/System/Volumes/") && text != "/System/Volumes/Data" {
            continue;
        }
        if text.starts_with("/System/Volumes/Data") && !out.is_empty() {
            continue;
        }
        // Application wrappers (for example an app that mounts its bundle at a
        // temporary path) are not disks a person can manage. Finder only shows
        // real volumes at `/` and under `/Volumes`.
        if cfg!(target_os = "macos")
            && text != "/"
            && !text.starts_with("/Volumes/")
            && !text.starts_with("/System/Volumes/")
        {
            continue;
        }
        if let Some((total, free)) = statvfs_for(&mount.mount_point) {
            let key = (total.as_u64(), free.as_u64());
            if seen_capacity.contains(&key) {
                continue;
            }
            seen_capacity.push(key);
            let is_root = mount.mount_point == root;
            let name = volume_name(&mount.mount_point, is_root);
            let is_removable = text.starts_with("/Volumes/")
                || mount.device.contains("/media/")
                || mount.device.starts_with("/dev/mmcblk");
            out.push(Volume {
                name,
                mount_point: mount.mount_point.clone(),
                total,
                free,
                is_root,
                is_removable,
                filesystem: mount.fstype.clone(),
            });
        }
    }
    out
}

fn volume_name(mount: &Path, is_root: bool) -> String {
    if is_root {
        return mac_volume_name(mount).unwrap_or_else(|| "Macintosh HD".into());
    }
    mac_volume_name(mount).unwrap_or_else(|| {
        mount
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| mount.display().to_string())
    })
}

/// Finder-style volume name on macOS (`diskutil info -plist` is the only cheap
/// and reliable source); other systems fall back to the mount point name.
fn mac_volume_name(mount: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let _ = mount;
    None
}

#[cfg(target_os = "macos")]
pub fn mount_entries() -> Vec<MountEntry> {
    let mut out = Vec::new();
    // statfs(2): getmntinfo returns a statically allocated array we own until
    // the next call. MNT_NOWAIT keeps it non-blocking.
    const MNT_NOWAIT: libc::c_int = 2;
    unsafe {
        let mut statfs_ptr: *mut libc::statfs = std::ptr::null_mut();
        let count = libc::getmntinfo(&mut statfs_ptr, MNT_NOWAIT);
        if count <= 0 || statfs_ptr.is_null() {
            return out;
        }
        for idx in 0..count as isize {
            let stat = *statfs_ptr.offset(idx);
            let mount_point = c_buffer_to_path(&stat.f_mntonname);
            let device = c_buffer_to_string(&stat.f_mntfromname);
            let fstype = c_buffer_to_string(&stat.f_fstypename);
            if mount_point.as_os_str().is_empty() {
                continue;
            }
            out.push(MountEntry {
                mount_point,
                device,
                fstype,
            });
        }
    }
    out
}

#[cfg(target_os = "linux")]
pub fn mount_entries() -> Vec<MountEntry> {
    let mut out = Vec::new();
    let text = match std::fs::read_to_string("/proc/self/mounts") {
        Ok(text) => text,
        Err(_) => return out,
    };
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(mount_point), Some(fstype)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let decoded = unescape_mount(mount_point);
        out.push(MountEntry {
            mount_point: PathBuf::from(decoded),
            device: device.to_string(),
            fstype: fstype.to_string(),
        });
    }
    out
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn mount_entries() -> Vec<MountEntry> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn unescape_mount(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match (chars.next(), chars.next(), chars.next()) {
                (Some('0'), Some('4'), Some('0')) => out.push(' '),
                (Some('0'), Some('1'), Some('2')) => out.push('\t'),
                (Some('0'), Some('1'), Some('1')) => out.push('\n'),
                (Some('1'), Some('3'), Some('4')) => out.push('\\'),
                (a, b, c) => {
                    for part in [a, b, c].into_iter().flatten() {
                        out.push(part);
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn c_buffer_to_string(buf: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .map(|b| *b as u8)
        .take_while(|b| *b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn c_buffer_to_path(buf: &[libc::c_char]) -> PathBuf {
    OsStrBytes(c_buffer_to_string(buf)).into_path()
}

struct OsStrBytes(String);
impl OsStrBytes {
    fn into_path(self) -> PathBuf {
        PathBuf::from(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_root_volume() {
        let volumes = volumes_from_mounts(&mount_entries());
        assert!(
            volumes.iter().any(|v| v.is_root),
            "expected a root volume, got {volumes:?}"
        );
        for volume in &volumes {
            assert!(volume.total.as_u64() > 0);
            assert!(volume.free <= volume.total);
        }
    }

    #[test]
    fn pseudo_filesystems_are_skipped() {
        let mounts = vec![MountEntry {
            mount_point: PathBuf::from("/dev"),
            device: "devfs".into(),
            fstype: "devfs".into(),
        }];
        assert!(volumes_from_mounts(&mounts).is_empty());
    }

    #[test]
    fn app_wrapper_mounts_are_not_volumes() {
        let mount_point = std::env::temp_dir();
        let mounts = vec![MountEntry {
            mount_point: mount_point.clone(),
            device: "/Applications/X.app/Wrapper".into(),
            fstype: "nullfs".into(),
        }];
        assert!(
            volumes_from_mounts(&mounts).is_empty(),
            "a nullfs app wrapper at {} must not look like a disk",
            mount_point.display()
        );
    }
}
