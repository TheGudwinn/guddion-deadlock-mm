//! Safe extraction of `.vpk` files from zip, 7z and rar archives.
//!
//! The format is detected from the file's magic bytes, not its extension. Every archive goes
//! through the same two passes: (1) validate all entries without writing anything (path
//! traversal, forbidden file types, encryption, limits); (2) extract only `.vpk` files under names
//! we choose ourselves, so nothing from the archive ever decides where a file is written.
use super::error::InstallError;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_entries: 5_000, max_total_bytes: 8 << 30, max_file_bytes: 4 << 30 }
    }
}

/// Executable / script types: an archive containing any of these is rejected outright.
const FORBIDDEN_EXT: &[&str] = &[
    "exe", "dll", "bat", "cmd", "com", "ps1", "psm1", "ps1xml", "msi", "msp", "mst", "scr", "lnk", "url", "scf", "inf",
    "vbs", "vbe", "js", "jse", "mjs", "cjs", "wsf", "wsh", "jar", "sh", "py", "pl", "rb", "php", "reg", "hta", "cpl",
    "msc", "pif", "gadget", "appx", "msix", "application", "sys", "drv", "ocx",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    SevenZ,
    Rar,
    Vpk,
}

pub fn detect_kind(path: &Path) -> Result<ArchiveKind, InstallError> {
    let mut head = [0u8; 8];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    let h = &head[..n];
    if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
        Ok(ArchiveKind::Zip)
    } else if h.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        Ok(ArchiveKind::SevenZ)
    } else if h.starts_with(b"Rar!\x1A\x07") {
        Ok(ArchiveKind::Rar)
    } else if h.starts_with(&0x55AA_1234u32.to_le_bytes()) {
        Ok(ArchiveKind::Vpk)
    } else {
        Err(InstallError::BadArchive("unrecognized file format (expected zip, 7z, rar or vpk)".into()))
    }
}

fn is_unsafe_name(name: &str) -> bool {
    let n = name.replace('\\', "/");
    n.starts_with('/') || n.contains(':') || n.split('/').any(|c| c == "..")
}

/// Windows ignores trailing dots and spaces, so `evil.exe.` is `evil.exe` on disk.
fn file_part(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or("").trim_end_matches(['.', ' '])
}

fn ext_of(name: &str) -> String {
    file_part(name).rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// True when ANY dot-separated part after the first is an executable type, so double extensions
/// such as `x.exe.vpk` or `x.vbs.txt` are caught as well as `x.exe`.
fn has_forbidden_ext(name: &str) -> bool {
    file_part(name).split('.').skip(1).any(|part| FORBIDDEN_EXT.contains(&part.trim().to_ascii_lowercase().as_str()))
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file name that is safe to create on Windows and Unix: no reserved device names, no characters
/// Windows rejects, no trailing dots or spaces, bounded length.
fn base_name(name: &str) -> String {
    let raw = file_part(name);
    let mut n: String = raw.chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    if n.is_empty() {
        n = "mod.vpk".into();
    }
    if n.chars().count() > 120 {
        let ext = ext_of(&n);
        n = format!("{}.{ext}", n.chars().take(100).collect::<String>());
    }
    let stem = n.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        n.insert(0, '_');
    }
    n
}

fn unique_dest(dir: &Path, base: &str) -> PathBuf {
    let mut p = dir.join(base);
    let mut n = 1;
    while p.exists() {
        p = dir.join(format!("{n}_{base}"));
        n += 1;
    }
    p
}

/// Running totals for pass 1.
struct Scan<'a> {
    limits: &'a Limits,
    entries: usize,
    declared_total: u64,
}

impl<'a> Scan<'a> {
    fn new(limits: &'a Limits) -> Self {
        Self { limits, entries: 0, declared_total: 0 }
    }

    /// Validate one entry. Returns true when it is a `.vpk` file that should be extracted.
    fn check(&mut self, name: &str, size: u64, is_dir: bool) -> Result<bool, InstallError> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(InstallError::TooManyEntries(self.entries));
        }
        if is_unsafe_name(name) {
            return Err(InstallError::UnsafePath(name.to_string()));
        }
        if is_dir {
            return Ok(false);
        }
        if has_forbidden_ext(name) {
            return Err(InstallError::ForbiddenFile(name.to_string()));
        }
        if ext_of(name) != "vpk" {
            return Ok(false);
        }
        if size > self.limits.max_file_bytes {
            return Err(InstallError::TooLarge);
        }
        self.declared_total = self.declared_total.saturating_add(size);
        if self.declared_total > self.limits.max_total_bytes {
            return Err(InstallError::TooLarge);
        }
        Ok(true)
    }
}

/// Copy at most `max + 1` bytes and fail if the entry turned out larger than allowed. `max` is the
/// smaller of the per-file limit and what is left of the total budget, so the disk never holds
/// more than the total limit even for a moment. Never overwrites an existing file.
fn copy_capped(src: &mut dyn Read, dest: &Path, max: u64) -> Result<u64, InstallError> {
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(dest)?;
    let written = std::io::copy(&mut src.take(max.saturating_add(1)), &mut f)?;
    f.flush()?;
    if written > max {
        drop(f);
        let _ = std::fs::remove_file(dest);
        return Err(InstallError::TooLarge);
    }
    Ok(written)
}

/// How many bytes the next file may take: the per-file cap, bounded by the remaining total budget.
fn budget(limits: &Limits, used: u64) -> u64 {
    limits.max_file_bytes.min(limits.max_total_bytes.saturating_sub(used))
}

/// Extract the `.vpk` files of a zip / 7z / rar archive into `out_dir` (flattened).
///
/// On any failure every file written so far is deleted again, so a rejected archive leaves nothing behind.
pub fn extract_vpks(archive: &Path, out_dir: &Path, limits: &Limits) -> Result<Vec<PathBuf>, InstallError> {
    let kind = detect_kind(archive)?;
    let mut out: Vec<PathBuf> = Vec::new();
    let result = match kind {
        ArchiveKind::Zip => extract_zip(archive, out_dir, limits, &mut out),
        ArchiveKind::SevenZ => extract_7z(archive, out_dir, limits, &mut out),
        ArchiveKind::Rar => extract_rar(archive, out_dir, limits, &mut out),
        ArchiveKind::Vpk => Err(InstallError::BadArchive("a bare VPK is not an archive".into())),
    }
    .and_then(|()| if out.is_empty() { Err(InstallError::NoVpk) } else { Ok(()) });
    match result {
        Ok(()) => Ok(out),
        Err(e) => {
            for p in &out {
                let _ = std::fs::remove_file(p);
            }
            Err(e)
        }
    }
}

fn bad(e: impl std::fmt::Display) -> InstallError {
    InstallError::BadArchive(e.to_string())
}

// ---- zip ---------------------------------------------------------------------------------------

fn zip_name<R: Read + ?Sized>(e: &zip::read::ZipFile<'_, R>) -> Result<String, InstallError> {
    e.name().map(|n| n.into_owned()).map_err(bad)
}

fn extract_zip(archive: &Path, out_dir: &Path, limits: &Limits, out: &mut Vec<PathBuf>) -> Result<(), InstallError> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(bad)?;
    if zip.len() > limits.max_entries {
        return Err(InstallError::TooManyEntries(zip.len()));
    }
    let mut scan = Scan::new(limits);
    for i in 0..zip.len() {
        let e = zip.by_index(i).map_err(bad)?;
        let name = zip_name(&e)?;
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(InstallError::UnsafePath(name)); // symlink
        }
        if e.encrypted() {
            return Err(InstallError::Encrypted);
        }
        scan.check(&name, e.size(), e.is_dir())?;
    }

    std::fs::create_dir_all(out_dir)?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(bad)?;
        let name = zip_name(&e)?;
        if e.is_dir() || ext_of(&name) != "vpk" {
            continue;
        }
        let dest = unique_dest(out_dir, &base_name(&name));
        total += copy_capped(&mut e, &dest, budget(limits, total))?;
        out.push(dest);
    }
    Ok(())
}

// ---- 7z ----------------------------------------------------------------------------------------

fn extract_7z(archive: &Path, out_dir: &Path, limits: &Limits, out: &mut Vec<PathBuf>) -> Result<(), InstallError> {
    use sevenz_rust2::{Archive, ArchiveReader, Password};

    let meta = Archive::open(archive).map_err(bad)?;
    let mut scan = Scan::new(limits);
    for f in &meta.files {
        scan.check(f.name(), f.size(), f.is_directory())?;
    }

    std::fs::create_dir_all(out_dir)?;
    let mut reader = ArchiveReader::open(archive, Password::empty()).map_err(bad)?;
    let mut total = 0u64;
    let mut failure: Option<InstallError> = None;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory() || !entry.has_stream() || ext_of(entry.name()) != "vpk" {
                return Ok(true); // skipped entries are still consumed correctly by the reader
            }
            let dest = unique_dest(out_dir, &base_name(entry.name()));
            match copy_capped(data, &dest, budget(limits, total)) {
                Ok(n) => {
                    total += n;
                    out.push(dest);
                    Ok(true)
                }
                Err(e) => {
                    failure = Some(e);
                    Ok(false)
                }
            }
        })
        .map_err(bad)?;
    match failure {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

// ---- rar ---------------------------------------------------------------------------------------

fn extract_rar(archive: &Path, out_dir: &Path, limits: &Limits, out: &mut Vec<PathBuf>) -> Result<(), InstallError> {
    let mut scan = Scan::new(limits);
    let listing = unrar::Archive::new(archive).open_for_listing().map_err(bad)?;
    for entry in listing {
        let h = entry.map_err(bad)?;
        if h.is_encrypted() {
            return Err(InstallError::Encrypted);
        }
        if h.is_split() {
            return Err(InstallError::BadArchive("multi-volume archives are not supported".into()));
        }
        let name = h.filename.to_string_lossy().replace('\\', "/");
        scan.check(&name, h.unpacked_size, h.is_directory())?;
    }

    std::fs::create_dir_all(out_dir)?;
    let mut total = 0u64;
    let mut cursor = unrar::Archive::new(archive).open_for_processing().map_err(bad)?;
    while let Some(header) = cursor.read_header().map_err(bad)? {
        let name = header.entry().filename.to_string_lossy().replace('\\', "/");
        if header.entry().is_file() && ext_of(&name) == "vpk" {
            let dest = unique_dest(out_dir, &base_name(&name));
            // The unrar library writes the whole entry itself, so it cannot be capped while writing;
            // track the path first so a failure (even mid-write) is cleaned up, then check the real size.
            out.push(dest.clone());
            cursor = header.extract_to(&dest).map_err(bad)?;
            let size = std::fs::metadata(&dest)?.len();
            total += size;
            if size > limits.max_file_bytes || total > limits.max_total_bytes {
                return Err(InstallError::TooLarge);
            }
        } else {
            cursor = header.skip().map_err(bad)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_and_trailing_extensions_are_forbidden() {
        for n in ["a.exe", "dir/a.EXE", "a.exe.vpk", "a.vbs.txt", "a.exe.", "a.exe ", "a.ps1.", "x.url"] {
            assert!(has_forbidden_ext(n), "{n}");
        }
        for n in ["a.vpk", "pak01_dir.vpk", "readme.txt", "my.mod.v2.vpk", "image.png"] {
            assert!(!has_forbidden_ext(n), "{n}");
        }
    }

    #[test]
    fn unsafe_names_are_rejected() {
        for n in ["../a.vpk", "a/../b.vpk", "/etc/a.vpk", "C:/a.vpk", r"a\..\b.vpk", "a.vpk:stream"] {
            assert!(is_unsafe_name(n), "{n}");
        }
        assert!(!is_unsafe_name("maps/a.vpk"));
    }

    #[test]
    fn output_names_are_safe_on_windows() {
        assert_eq!(base_name("dir/sub/mod.vpk"), "mod.vpk");
        assert_eq!(base_name("CON.vpk"), "_CON.vpk");
        assert_eq!(base_name("nul"), "_nul");
        assert_eq!(base_name("a<b>.vpk"), "a_b_.vpk");
        assert_eq!(base_name(""), "mod.vpk");
        assert!(base_name(&format!("{}.vpk", "x".repeat(300))).chars().count() <= 110);
    }

    #[test]
    fn budget_never_exceeds_the_total() {
        let l = Limits { max_entries: 10, max_total_bytes: 100, max_file_bytes: 80 };
        assert_eq!(budget(&l, 0), 80);
        assert_eq!(budget(&l, 50), 50);
        assert_eq!(budget(&l, 100), 0);
        assert_eq!(budget(&l, 500), 0);
    }

    #[test]
    fn copy_capped_refuses_oversize_and_existing_files() {
        let dir = std::env::temp_dir().join(format!("ddmm-ex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("a.vpk");
        assert!(matches!(copy_capped(&mut &[0u8; 10][..], &dest, 5), Err(InstallError::TooLarge)));
        assert!(!dest.exists(), "oversized file is removed");
        assert_eq!(copy_capped(&mut &[0u8; 5][..], &dest, 5).unwrap(), 5);
        assert!(copy_capped(&mut &[0u8; 1][..], &dest, 5).is_err(), "never overwrites");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
