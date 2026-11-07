//! Who signed this file, as far as the operating system will say.
//!
//! The answer is used in two places: the startup list colours an entry that
//! nobody vouches for, and the scanner treats "unsigned binary in a user-writable
//! path that runs at login" as evidence. It is deliberately not treated as a
//! verdict. An unsigned file is ordinary for anything a person built themselves,
//! and a signed file is ordinary for adware that bought a certificate, so the
//! UI says *unsigned* rather than *suspicious* and leaves the reasoning to the
//! reader.
//!
//! macOS asks `codesign`, which is the same tool the release scripts use, so the
//! app and the release check cannot disagree about what "ad-hoc signed" means.
//! Windows asks the Authenticode API directly, because the PowerShell equivalent
//! costs a second to start and the process table asks this question per row.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the signature says about one file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trust {
    /// A signature exists and the operating system considers it intact.
    pub signed: bool,
    /// Ad-hoc / link-time signature: bytes are sealed, but nobody is named.
    pub ad_hoc: bool,
    /// Authenticode is present but chain validation failed (expired, unknown CA).
    pub untrusted: bool,
    /// Vendor named by the leaf certificate.
    pub signer: Option<String>,
    /// Why there is no signature, when that is worth saying.
    pub note: Option<String>,
}

impl Trust {
    /// One word for a table column.
    pub fn state(&self) -> &'static str {
        if self.signed && !self.ad_hoc && !self.untrusted {
            "trusted"
        } else if self.signed {
            "adhoc"
        } else if self.untrusted {
            "untrusted"
        } else {
            "unsigned"
        }
    }
}

/// Ask the operating system about one file. Never fails: no answer is a normal
/// answer, and the caller decides whether silence is worth showing.
///
/// Answers are remembered for the life of the process, keyed by path plus the
/// file's size and modification time, because both callers ask in bulk: the
/// startup list asks once per entry and the process table once per distinct
/// executable on screen. The question itself costs a subprocess on macOS and a
/// certificate-chain walk on Windows, so one refresh of the process tab would be
/// hundreds of them. A file that changes is looked up again, which is why the
/// identity is part of the key and not the path alone.
pub fn assess(path: &Path) -> Trust {
    let Some(key) = identity(path) else {
        // Nothing to key on. Asking anyway is what turns "the file is not there"
        // into an answer instead of a cached guess.
        return assess_uncached(path);
    };
    let mut cache = cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(found) = cache.get(&key) {
        return found.clone();
    }
    let trust = assess_uncached(path);
    // Bounded, because a utility that grows its own footprint to answer a
    // display question is its own kind of junk. Half is dropped rather than all,
    // so a table that is still on screen keeps the answers it is showing.
    if cache.len() > 4_000 {
        let stale: Vec<_> = cache.keys().take(cache.len() / 2).cloned().collect();
        for stale in stale {
            cache.remove(&stale);
        }
    }
    cache.insert(key, trust.clone());
    trust
}

type CacheKey = (std::path::PathBuf, u64, u128);

fn identity(path: &Path) -> Option<CacheKey> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    Some((path.to_path_buf(), metadata.len(), modified))
}

fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<CacheKey, Trust>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<CacheKey, Trust>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn assess_uncached(path: &Path) -> Trust {
    #[cfg(target_os = "macos")]
    {
        macos::assess(path)
    }
    #[cfg(windows)]
    {
        windows::assess(path)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Trust::default()
    }
}

/// Whether the file carries a signature *blob*, without asking the operating
/// system to validate it.
///
/// This is the cheap sibling of [`assess`]: it reads the Mach-O load commands or
/// the PE certificate directory and stops, so it can run over every row of a
/// process table or every candidate in a full scan. What it cannot tell you is
/// whether that signature is trusted, which is exactly the difference a person
/// needs when they open the row — and that is why the detail view still asks
/// `assess` and this result is only ever shown as "signed by somebody" or "no
/// signature at all".
pub fn has_signature_blob(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        macho_has_signature(path)
    }
    #[cfg(windows)]
    {
        pe_has_certificate(path)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        false
    }
}

/// Scan the load commands for `LC_CODE_SIGNATURE` (0x1D), in either endianness,
/// and into each slice of a fat binary.
#[cfg(target_os = "macos")]
fn macho_has_signature(path: &Path) -> bool {
    // Only the head of the image is ever consulted: a Mach-O header plus a
    // thousand load commands is a few tens of kilobytes, and a fat binary keeps
    // its first slice within the first megabyte. Reading a four-gigabyte disk
    // image whole to answer one yes/no question is the kind of cost that gets a
    // scanner switched off.
    const WINDOW: u64 = 2 * 1024 * 1024;
    let Ok(length) = std::fs::metadata(path).map(|metadata| metadata.len()) else {
        return false;
    };
    if length < 32 {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::with_capacity((WINDOW).min(length) as usize);
    {
        use std::io::Read;
        if Read::by_ref(&mut file)
            .take(WINDOW)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return false;
        }
    }
    if bytes.len() < 32 {
        return false;
    }
    let word = |bytes: &[u8], offset: usize, big: bool| -> Option<u32> {
        let window = bytes.get(offset..offset + 4)?;
        Some(if big {
            u32::from_be_bytes(window.try_into().ok()?)
        } else {
            u32::from_le_bytes(window.try_into().ok()?)
        })
    };
    let head = [bytes[0], bytes[1], bytes[2], bytes[3]];
    const FAT_32: [u8; 4] = [0xCA, 0xFE, 0xBA, 0xBE];
    const FAT_SWAPPED: [u8; 4] = [0xBE, 0xBA, 0xFE, 0xCA];
    if head == FAT_32 || head == FAT_SWAPPED {
        // The fat header is always big-endian in practice; the spelling of its
        // magic says which order it was actually written in, and that governs
        // only the table below — each slice re-declares its own endianness.
        let big = head == FAT_32;
        let count = word(&bytes, 4, big).unwrap_or(0).min(32) as usize;
        for index in 0..count {
            // fat_arch: cputype, cpusubtype, offset, size, align.
            let Some(offset) = word(&bytes, 8 + index * 20 + 8, big) else {
                continue;
            };
            // Each slice carries its own magic, and therefore its own
            // endianness: a fat header written big-endian normally contains
            // little-endian arm64 and x86_64 images.
            if macho_slice_signed(&bytes, offset as usize, &word) {
                return true;
            }
        }
        return false;
    }
    macho_slice_signed(&bytes, 0, &word)
}

/// Walk one Mach-O header's load commands looking for the signature command.
#[cfg(target_os = "macos")]
fn macho_slice_signed(
    bytes: &[u8],
    start: usize,
    word: &impl Fn(&[u8], usize, bool) -> Option<u32>,
) -> bool {
    const LC_CODE_SIGNATURE: u32 = 0x1D;
    // A 64-bit little-endian image begins `CF FA ED FE` — the magic 0xFEEDFACF
    // read back as little-endian — and a big-endian one begins with the same
    // four bytes in reverse. The 32-bit pair differs by one byte. Which spelling
    // appeared is the only thing that decides the endianness of every field
    // after it, so the table says it directly instead of inferring it from
    // wherever this header was reached from.
    // Which magic appeared decides two things at once: the endianness of every
    // field after it, and whether this is a 64-bit header (32 bytes) or a
    // 32-bit one (28 bytes). Getting either wrong reads the load commands from
    // the wrong address and reports a signed binary as unsigned.
    const THIN: [([u8; 4], bool, usize); 4] = [
        ([0xCF, 0xFA, 0xED, 0xFE], false, 32), // MH_MAGIC_64, host order
        ([0xCE, 0xFA, 0xED, 0xFE], false, 28), // MH_MAGIC,   host order
        ([0xFE, 0xED, 0xFA, 0xCF], true, 32),  // MH_MAGIC_64, swapped
        ([0xFE, 0xED, 0xFA, 0xCE], true, 28),  // MH_MAGIC,   swapped
    ];
    let Some(head) = bytes.get(start..start + 4) else {
        return false;
    };
    let Ok(head): Result<[u8; 4], _> = head.try_into() else {
        return false;
    };
    let Some((_, big, header)) = THIN.iter().find(|(magic, _, _)| *magic == head) else {
        return false;
    };
    let (big, header) = (*big, *header);
    // A corrupt fat header can point a slice past what was actually read.
    if start + header > bytes.len() {
        return false;
    }
    // mach_header: magic, cputype, cpusubtype, filetype, ncmds, sizeofcmds…
    // `ncmds` is the fifth field, at offset 16 — reading 12 instead finds
    // `filetype`, which is how every signed binary looks unsigned.
    let commands = word(bytes, start + 16, big).unwrap_or(0).min(4096) as usize;
    let mut cursor = start + header;
    for _ in 0..commands {
        let Some(command) = word(bytes, cursor, big) else {
            return false;
        };
        // High bit marks a 64-bit-only command; the number underneath is the same.
        if command & 0x7FFF_FFFF == LC_CODE_SIGNATURE {
            return true;
        }
        let Some(size) = word(bytes, cursor + 4, big) else {
            return false;
        };
        if size < 8 {
            return false;
        }
        cursor += size as usize;
        if cursor + 8 > bytes.len() {
            return false;
        }
    }
    false
}

/// A PE file's certificate table is data directory entry 4: an offset into the
/// file, not a relative virtual address, which makes it a four-byte read once
/// the header has been walked.
#[cfg(windows)]
fn pe_has_certificate(path: &Path) -> bool {
    pe_has_certificate_at(path).unwrap_or(false)
}

/// The same walk written against `Option`, so an out-of-range field is a plain
/// "no" rather than a panic. Split out because `?` in the `bool`-returning
/// wrapper above does not compile, and this half only builds on Windows, so the
/// mistake is invisible to a macOS `cargo check`.
#[cfg(windows)]
fn pe_has_certificate_at(path: &Path) -> Option<bool> {
    use std::io::Read;
    let length = std::fs::metadata(path).ok()?.len();
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0_u8; 4096];
    let read = file.read(&mut head).ok()?;
    let u32_at = |offset: usize| -> Option<u32> {
        let window = head.get(offset..offset + 4)?;
        Some(u32::from_le_bytes(window.try_into().ok()?))
    };
    if read < 64 || &head[0..2] != b"MZ" {
        return Some(false);
    }
    let pe = u32_at(0x3C)? as usize;
    if pe + 24 > read || &head[pe..pe + 4] != b"PE\0\0" {
        return Some(false);
    }
    let optional = pe + 24;
    let magic = u32_at(optional)? & 0xFFFF;
    // Data directories start after the optional header; their offset differs
    // between PE32 and PE32+, which is what the magic byte says.
    let directories = optional + if magic == 0x20B { 112 } else { 96 };
    let entry = u32_at(directories + 4 * 8)?;
    let size = u32_at(directories + 4 * 8 + 4)?;
    Some(entry != 0 && size != 0 && (entry as u64) < length)
}

/// Cheap gate for a table with hundreds of rows: only Mach-O and PE files can
/// carry a signature, so everything else is answered without a subprocess.
pub fn might_be_executable(path: &Path) -> bool {
    if path.is_dir() {
        return false;
    }
    if path.extension().is_some() {
        const SIGNED_KINDS: [&str; 9] = [
            "app",
            "framework",
            "bundle",
            "xpc",
            "dylib",
            "so",
            "exe",
            "dll",
            "sys",
        ];
        let ext = path
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        return SIGNED_KINDS.contains(&ext.as_str()) || ext.is_empty();
    }
    // Extension-less binaries are the norm on macOS, so the header decides.
    header_is_executable(path)
}

fn header_is_executable(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0_u8; 8];
    let Ok(_) = file.read(&mut head) else {
        return false;
    };
    // Mach-O 32/64, fat binary, and their byte-swapped forms.
    const MACHOS: [[u8; 4]; 6] = [
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xbe, 0xba, 0xfe, 0xca],
    ];
    if MACHOS.iter().any(|magic| head.starts_with(magic)) {
        return true;
    }
    // PE: `MZ` at the front, `PE\0\0` wherever the header table points.
    head.starts_with(b"MZ")
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;

    const CODESIGN: &str = "/usr/bin/codesign";

    /// Resolves an `.app` to the binary `codesign` should be asked about, since
    /// asking about the bundle is what `--verify --deep` would do anyway.
    fn executable_of(path: &Path) -> PathBuf {
        let candidate = path.join("Contents/MacOS");
        let Ok(entries) = std::fs::read_dir(&candidate) else {
            return path.to_path_buf();
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_file())
            .unwrap_or_else(|| path.to_path_buf())
    }

    pub fn assess(path: &Path) -> Trust {
        if !path.exists() {
            return Trust {
                note: Some("the file is not there".into()),
                ..Default::default()
            };
        }
        if !super::might_be_executable(path) {
            return Trust::default();
        }
        let target = if path.is_dir() {
            executable_of(path)
        } else {
            path.to_path_buf()
        };
        let Ok(output) = Command::new(CODESIGN)
            .args(["-dvv", &target.display().to_string()])
            .output()
        else {
            return Trust {
                note: Some("codesign could not run".into()),
                ..Default::default()
            };
        };
        // `codesign -dvv` writes its report to stderr and exits non-zero for
        // unsigned files, which is an answer rather than a failure.
        let report = String::from_utf8_lossy(&output.stderr).into_owned();
        if report.contains("code object is not signed at all") || report.is_empty() {
            return Trust {
                note: (!report.is_empty()).then(|| "not signed".into()),
                ..Default::default()
            };
        }

        let signer = report
            .lines()
            .find_map(|line| line.strip_prefix("Authority="))
            .map(|name| name.trim().to_string());
        let team = report
            .lines()
            .find_map(|line| line.strip_prefix("TeamIdentifier="))
            .map(|value| value.trim().to_string());
        let ad_hoc = team.as_deref() == Some("not set") && signer.is_none();
        let signed = signer.is_some()
            || report.contains("CodeDirectory") && !ad_hoc
            || report.contains("Signature size");

        if ad_hoc {
            return Trust {
                signed: true,
                ad_hoc: true,
                note: Some("ad-hoc signed — the bytes are sealed, nobody is named".into()),
                ..Default::default()
            };
        }
        if !signed {
            return Trust {
                note: Some("no usable signature".into()),
                ..Default::default()
            };
        }
        Trust {
            signed: true,
            signer: signer.or(team),
            ..Default::default()
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    /// Ask Authenticode through `WinVerifyTrust`. The status codes are read so
    /// an expired or unrooted certificate reports as *untrusted* rather than as
    /// unsigned: those files were signed by somebody, and a person deciding
    /// whether to quarantine them deserves to know that much.
    pub fn assess(path: &Path) -> Trust {
        if !super::might_be_executable(path) {
            return Trust::default();
        }
        if !path.exists() {
            return Trust {
                note: Some("the file is not there".into()),
                ..Default::default()
            };
        }
        match authenticode(path) {
            Ok(()) => Trust {
                signed: true,
                signer: publisher(path),
                ..Default::default()
            },
            Err(status) => {
                const NO_SIGNATURE: [u32; 2] = [0x800b0100, 0xc0000225];
                let present = !NO_SIGNATURE.contains(&status);
                Trust {
                    untrusted: present,
                    signer: present.then(|| publisher(path)).flatten(),
                    note: Some(format!("Authenticode reported 0x{status:08x}")),
                    ..Default::default()
                }
            }
        }
    }

    fn authenticode(path: &Path) -> Result<(), u32> {
        use windows_sys::core::GUID;
        use windows_sys::Win32::Security::WinTrust::{
            WinVerifyTrust, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO, WTD_CHOICE_FILE,
            WTD_REVOKE_NONE, WTD_STATEACTION_IGNORE, WTD_UI_NONE,
        };

        // WINTRUST_ACTION_GENERIC_VERIFY_V2
        const ACTION: GUID = GUID::from_u128(0x00aa003c_0000_0000_c000_000000000046);

        let wide = wide_path(path);
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: wide.as_ptr(),
            hFile: std::ptr::null_mut(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            pPolicyCallbackData: std::ptr::null_mut(),
            pSIPClientData: std::ptr::null_mut(),
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_IGNORE,
            hWVTStateData: std::ptr::null_mut(),
            pwszURLReference: std::ptr::null_mut(),
            dwProvFlags: 0,
            dwUIContext: 0,
            pSignatureSettings: std::ptr::null_mut(),
        };
        let status = unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut ACTION.clone() as *mut GUID,
                &mut data as *mut _ as *mut std::ffi::c_void,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(status as u32)
        }
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    /// The `CompanyName` of the version resource: what Task Manager prints as
    /// Publisher, and a magnitude cheaper than parsing the certificate chain.
    fn publisher(path: &Path) -> Option<String> {
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
        };

        let wide = wide_path(path);
        let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut()) };
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0_u8; size as usize];
        let ok = unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) };
        if ok == 0 {
            return None;
        }

        let mut pointer: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut length = 0_u32;
        let translation = wide_str("\\VarFileInfo\\Translation");
        let found = unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                translation.as_ptr(),
                &mut pointer,
                &mut length,
            )
        };
        let (language, code_page) = if found == 0 || length < 4 {
            (0x0409, 1200)
        } else {
            let pairs = unsafe {
                std::slice::from_raw_parts(pointer.cast::<(u16, u16)>(), (length / 4) as usize)
            };
            pairs.first().copied().unwrap_or((0x0409, 1200))
        };

        let key = wide_str(&format!("\\StringInfo\\{language:04x}{code_page:04x}"));
        let found = unsafe {
            VerQueryValueW(
                buffer.as_ptr().cast(),
                key.as_ptr(),
                &mut pointer,
                &mut length,
            )
        };
        if found == 0 || length == 0 {
            return None;
        }
        let block =
            unsafe { std::slice::from_raw_parts(pointer.cast::<u16>(), (length / 2) as usize) };
        string_value(block, "CompanyName")
    }

    fn wide_str(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The `StringFileInfo` block is `key\0value\0key\0value\0…` in UTF-16,
    /// terminated by an empty key.
    fn string_value(block: &[u16], wanted: &str) -> Option<String> {
        let mut entries: Vec<String> = Vec::new();
        let mut current: Vec<u16> = Vec::new();
        for unit in block {
            if *unit == 0 {
                if !current.is_empty() {
                    let taken: Vec<u16> = std::mem::take(&mut current);
                    entries.push(String::from_utf16_lossy(&taken));
                }
            } else {
                current.push(*unit);
            }
        }
        for pair in entries.as_chunks::<2>().0 {
            if pair[0].eq_ignore_ascii_case(wanted) && !pair[1].is_empty() {
                return Some(pair[1].clone());
            }
        }
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod blob_tests {
    use super::*;

    /// Apple's own binaries are signed, and these three are on every macOS
    /// version this runs on, so the header check has something fixed to point at.
    #[test]
    fn a_signed_system_binary_reports_its_signature_blob() {
        for path in ["/bin/ls", "/bin/cat", "/usr/lib/dyld"] {
            let file = std::path::Path::new(path);
            if !file.is_file() {
                continue;
            }
            assert!(
                has_signature_blob(file),
                "{path} is signed, and the load commands say so"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_macho_reports_no_signature() {
        let dir = std::env::temp_dir().join(format!("squid-trust-blob-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let plain = dir.join("plain");
        std::fs::write(&plain, b"#!/bin/sh\necho not a binary\n").unwrap();
        assert!(!has_signature_blob(&plain));

        // A Mach-O magic with nothing behind it: the header is too short to hold
        // any load commands, and reading past the end must not panic.
        let truncated = dir.join("truncated");
        std::fs::write(&truncated, [0xCF_u8, 0xFA, 0xED, 0xFE, 0, 0, 0, 1]).unwrap();
        assert!(!has_signature_blob(&truncated));

        // A fat header that points its only slice past the end of the file.
        let lying = dir.join("lying-fat");
        let mut bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 1];
        bytes.extend([0_u8; 8]);
        bytes.extend((1_u32 << 20).to_be_bytes());
        bytes.extend([0_u8; 8]);
        std::fs::write(&lying, &bytes).unwrap();
        assert!(!has_signature_blob(&lying));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The strongest form of this test is the same file on both sides of the
    /// signature boundary. A freshly linked arm64 binary is already
    /// `adhoc, linker-signed`, so "just compiled" cannot prove anything; removing
    /// and re-adding the signature shows the answer follows the load command
    /// rather than the file's name, size, or the parser's mood.
    #[test]
    fn the_answer_follows_the_load_command() {
        let dir = std::env::temp_dir().join(format!("squid-trust-cc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("probe.c");
        std::fs::write(&source, "int main(void){return 0;}\n").unwrap();
        let binary = dir.join("probe");
        let run = |program: &str, args: &[&str]| {
            std::process::Command::new(program)
                .args(args)
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        };
        if !run(
            "cc",
            &["-o", &binary.to_string_lossy(), &source.to_string_lossy()],
        ) {
            // No compiler here; the shape of the answer is still covered by the
            // non-Mach-O test above.
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        if !run(
            "codesign",
            &["--remove-signature", &binary.to_string_lossy()],
        ) {
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        assert!(
            !has_signature_blob(&binary),
            "with the signature command removed, nothing should be reported"
        );
        assert!(
            run("codesign", &["-s", "-", &binary.to_string_lossy()]),
            "ad-hoc signing the probe should succeed"
        );
        assert!(
            has_signature_blob(&binary),
            "adding the signature command back must flip the answer"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
