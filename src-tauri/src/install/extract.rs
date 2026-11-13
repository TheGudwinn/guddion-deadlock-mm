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
    "exe", "dll", "bat", "cmd", "com", "ps1", "psm1", "msi", "scr", "lnk", "vbs", "vbe", "js", "jse", "jar", "sh", "py",
    "reg", "hta", "cpl",
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

fn ext_of(name: &str) -> String {
    name.rsplit(['/', '\\']).next().and_then(|f| f.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn base_name(name: &str) -> String {
    name.replace('\\', "/").rsplit('/').next().unwrap_or("mod.vpk").to_string()
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
        let ext = ext_of(name);
        if FORBIDDEN_EXT.contains(&ext.as_str()) {
            return Err(InstallError::ForbiddenFile(name.to_string()));
        }
        if ext != "vpk" {
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

/// Copy at most `max + 1` bytes and fail if the entry turned out larger than allowed.
fn copy_capped(src: &mut dyn Read, dest: &Path, max: u64) -> Result<u64, InstallError> {
    let mut f = std::fs::File::create(dest)?;
    let written = std::io::copy(&mut src.take(max + 1), &mut f)?;
    f.flush()?;
    if written > max {
        drop(f);
        let _ = std::fs::remove_file(dest);
        return Err(InstallError::TooLarge);
    }
    Ok(written)
}

/// Extract the `.vpk` files of a zip / 7z / rar archive into `out_dir` (flattened).
pub fn extract_vpks(archive: &Path, out_dir: &Path, limits: &Limits) -> Result<Vec<PathBuf>, InstallError> {
    let out = match detect_kind(archive)? {
        ArchiveKind::Zip => extract_zip(archive, out_dir, limits)?,
        ArchiveKind::SevenZ => extract_7z(archive, out_dir, limits)?,
        ArchiveKind::Rar => extract_rar(archive, out_dir, limits)?,
        ArchiveKind::Vpk => return Err(InstallError::BadArchive("a bare VPK is not an archive".into())),
    };
    if out.is_empty() {
        return Err(InstallError::NoVpk);
    }
    Ok(out)
}

fn bad(e: impl std::fmt::Display) -> InstallError {
    InstallError::BadArchive(e.to_string())
}

// ---- zip ---------------------------------------------------------------------------------------

fn zip_name<R: Read + ?Sized>(e: &zip::read::ZipFile<'_, R>) -> Result<String, InstallError> {
    e.name().map(|n| n.into_owned()).map_err(bad)
}

fn extract_zip(archive: &Path, out_dir: &Path, limits: &Limits) -> Result<Vec<PathBuf>, InstallError> {
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
    let mut out = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).map_err(bad)?;
        let name = zip_name(&e)?;
        if e.is_dir() || ext_of(&name) != "vpk" {
            continue;
        }
        let dest = unique_dest(out_dir, &base_name(&name));
        total += copy_capped(&mut e, &dest, limits.max_file_bytes)?;
        if total > limits.max_total_bytes {
            let _ = std::fs::remove_file(&dest);
            return Err(InstallError::TooLarge);
        }
        out.push(dest);
    }
    Ok(out)
}

// ---- 7z ----------------------------------------------------------------------------------------

fn extract_7z(archive: &Path, out_dir: &Path, limits: &Limits) -> Result<Vec<PathBuf>, InstallError> {
    use sevenz_rust2::{Archive, ArchiveReader, Password};

    let meta = Archive::open(archive).map_err(bad)?;
    let mut scan = Scan::new(limits);
    for f in &meta.files {
        scan.check(f.name(), f.size(), f.is_directory())?;
    }

    std::fs::create_dir_all(out_dir)?;
    let mut reader = ArchiveReader::open(archive, Password::empty()).map_err(bad)?;
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut failure: Option<InstallError> = None;
    reader
        .for_each_entries(|entry, data| {
            if entry.is_directory() || !entry.has_stream() || ext_of(entry.name()) != "vpk" {
                return Ok(true); // skipped entries are still consumed correctly by the reader
            }
            let dest = unique_dest(out_dir, &base_name(entry.name()));
            match copy_capped(data, &dest, limits.max_file_bytes) {
                Ok(n) => {
                    total += n;
                    out.push(dest);
                    if total > limits.max_total_bytes {
                        failure = Some(InstallError::TooLarge);
                        return Ok(false);
                    }
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
        None => Ok(out),
    }
}

// ---- rar ---------------------------------------------------------------------------------------

fn extract_rar(archive: &Path, out_dir: &Path, limits: &Limits) -> Result<Vec<PathBuf>, InstallError> {
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
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut cursor = unrar::Archive::new(archive).open_for_processing().map_err(bad)?;
    while let Some(header) = cursor.read_header().map_err(bad)? {
        let name = header.entry().filename.to_string_lossy().replace('\\', "/");
        if header.entry().is_file() && ext_of(&name) == "vpk" {
            let dest = unique_dest(out_dir, &base_name(&name));
            cursor = header.extract_to(&dest).map_err(bad)?;
            let size = std::fs::metadata(&dest)?.len();
            total += size;
            if size > limits.max_file_bytes || total > limits.max_total_bytes {
                let _ = std::fs::remove_file(&dest);
                return Err(InstallError::TooLarge);
            }
            out.push(dest);
        } else {
            cursor = header.skip().map_err(bad)?;
        }
    }
    Ok(out)
}
