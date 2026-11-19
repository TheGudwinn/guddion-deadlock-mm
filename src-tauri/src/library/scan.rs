//! Inspect what is already in `addons/`, including files installed by other managers.
use crate::install::pak_order::{is_ours, DISABLED_SUFFIX};
use crate::install::vpk::{read_vpk_index, VpkFile};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

type Index = Result<Vec<VpkFile>, String>;

/// Parsed VPK directories by path, valid while the file's size and modification time are unchanged. The
/// library, its statistics and the contents view all scan the folder; without this each scan re-read
/// the directory part of every VPK (tens to hundreds of files).
static INDEX_CACHE: LazyLock<Mutex<HashMap<PathBuf, (u64, Option<SystemTime>, Index)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn read_index_cached(path: &Path, size: u64, modified: Option<SystemTime>) -> Index {
    let mut cache = INDEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, m, idx)) = cache.get(path) {
        if *s == size && *m == modified {
            return idx.clone();
        }
    }
    let idx = read_vpk_index(path).map(|i| i.files);
    cache.insert(path.to_path_buf(), (size, modified, idx.clone()));
    idx
}

#[derive(Debug, Clone)]
pub struct ExistingAddon {
    pub file_name: String,
    pub path: PathBuf,
    /// Installed by this app (as opposed to another manager or by hand).
    pub ours: bool,
    /// The game does not load this file: switched off with our `.disabled` suffix, or another manager's
    /// file that is not named `pak##_dir.vpk` (that is how they switch a mod off).
    pub disabled: bool,
    pub size: u64,
    /// When the file was put here (creation time, unix seconds); a copy keeps the mtime of the original, this does not.
    pub installed_at: Option<i64>,
    /// Parsed VPK directory, or the reason it could not be read.
    pub entries: Result<Vec<VpkFile>, String>,
}

/// List every addon VPK directory file (`*_dir.vpk`), including switched-off ones (`*.disabled`).
pub fn scan_addons(addons_dir: &Path) -> std::io::Result<Vec<ExistingAddon>> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(addons_dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for ent in rd {
        let ent = ent?;
        if !ent.file_type()?.is_file() {
            continue;
        }
        let file_name = ent.file_name().to_string_lossy().into_owned();
        let base = file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(&file_name);
        if !base.ends_with("_dir.vpk") {
            continue; // .dmm.json, chunk files, etc.
        }
        let ours = is_ours(&file_name);
        let disabled = file_name.ends_with(DISABLED_SUFFIX) || (!ours && !crate::library::foreign_id::is_loaded_name(&file_name));
        let path = ent.path();
        let meta = ent.metadata()?;
        out.push(ExistingAddon {
            entries: read_index_cached(&path, meta.len(), meta.modified().ok()),
            size: meta.len(),
            installed_at: meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64),
            file_name,
            path,
            ours,
            disabled,
        });
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(out)
}

fn fingerprint(files: &[VpkFile]) -> BTreeSet<(&str, u32)> {
    files.iter().map(|f| (f.path.as_str(), f.crc)).collect()
}

/// An existing addon with exactly the same files and CRCs as `new` (same mod, possibly renamed).
pub fn find_duplicate<'a>(new: &[VpkFile], existing: &'a [ExistingAddon]) -> Option<&'a ExistingAddon> {
    if new.is_empty() {
        return None;
    }
    let fp = fingerprint(new);
    existing.iter().find(|a| a.entries.as_ref().is_ok_and(|e| fingerprint(e) == fp))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Overlap {
    pub path: String,
    pub with_addon: String,
    /// Same CRC: the copies are identical, so the overlap is harmless.
    pub identical: bool,
    /// The other addon is currently disabled.
    pub other_disabled: bool,
}

/// Files in `new` that an already installed addon also provides.
pub fn overlaps(new: &[VpkFile], existing: &[ExistingAddon]) -> Vec<Overlap> {
    let mine: HashMap<&str, u32> = new.iter().map(|f| (f.path.as_str(), f.crc)).collect();
    let mut out = Vec::new();
    for a in existing {
        let Ok(files) = &a.entries else { continue };
        for f in files {
            if let Some(&crc) = mine.get(f.path.as_str()) {
                out.push(Overlap {
                    path: f.path.clone(),
                    with_addon: a.file_name.clone(),
                    identical: crc == f.crc,
                    other_disabled: a.disabled,
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.with_addon, &a.path).cmp(&(&b.with_addon, &b.path)));
    out
}

/// Files provided by two or more addons the game loads whose contents differ (same path, different CRC).
/// Identical copies are harmless and disabled addons do not count. Unreadable addons are skipped.
pub fn real_conflicts(addons: &[ExistingAddon]) -> usize {
    use crate::library::conflicts::{find_conflicts, ModFiles};
    let mods: Vec<ModFiles> = addons
        .iter()
        .enumerate()
        // Only files the game actually loads can conflict (switched-off mods are marked `disabled`).
        .filter(|(_, a)| !a.disabled)
        .filter_map(|(i, a)| a.entries.as_ref().ok().map(|e| ModFiles { mod_id: i as i64, files: e.iter().map(|f| (f.path.clone(), f.crc)).collect() }))
        .collect();
    find_conflicts(&mods).iter().filter(|c| !c.identical).count()
}
