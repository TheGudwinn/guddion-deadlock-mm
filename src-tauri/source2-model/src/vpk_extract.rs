use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};


use crate::error::{Result, Source2Error};
use crate::preview_report::{
    PreviewDependency, PreviewEvidence, PreviewIssue, PreviewIssueKind, PreviewReport,
};

const VPK_SIGNATURE: u32 = 0x55aa1234;

struct EntryInfo {
    full_path: String,
    archive_index: u16,
    entry_offset: u32,
    entry_length: u32,
    preload: Vec<u8>,
}

/// Suffix of the sidecar that redirects companion-archive lookups: a file named
/// `<name>.vpk.origin` beside a VPK holds the absolute directory its `_NNN.vpk`
/// companions live in.
///
/// This exists for staged copies. A `_dir.vpk` can be copied somewhere else and
/// parsed from there, but copying its companions along is not always viable —
/// the base game's pak is a 6 MB directory file in front of ~100 companion
/// archives totalling tens of gigabytes. The sidecar lets the staged directory
/// file resolve entry payloads read-only from where the companions actually are.
pub const ORIGIN_SIDECAR_SUFFIX: &str = ".origin";

/// Path of the origin sidecar belonging to `vpk_path`.
pub fn origin_sidecar_path(vpk_path: &Path) -> PathBuf {
    let mut name = vpk_path.file_name().unwrap_or_default().to_os_string();
    name.push(ORIGIN_SIDECAR_SUFFIX);
    vpk_path.with_file_name(name)
}

/// Directory to resolve `_NNN.vpk` companions in: the one named by the origin
/// sidecar when it exists and still points at a directory, else the VPK's own.
pub fn companion_dir(vpk_path: &Path) -> PathBuf {
    if let Ok(text) = fs::read_to_string(origin_sidecar_path(vpk_path)) {
        let origin = PathBuf::from(text.trim());
        if origin.is_dir() {
            return origin;
        }
    }
    vpk_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub struct VpkArchive {
    path: PathBuf,
    companion_dir: PathBuf,
    data_section_start: usize,
    inline_data_length: Option<u32>,
    entries: Vec<EntryInfo>,
    entry_index: HashMap<String, usize>,
    /// An unpacked file tree that shadows the archive. When the Foundry has an
    /// editable workspace open, a file present there is the user's edited
    /// version of that entry, so previews must read it instead of the packed
    /// original.
    overlay_dir: Option<PathBuf>,
    fallbacks: Vec<Arc<VpkArchive>>,
    evidence: Option<Mutex<PreviewEvidence>>,
}

impl VpkArchive {
    pub fn open(vpk_path: &Path) -> Result<Self> {
        let mut file = fs::File::open(vpk_path)?;
        let mut buffer = [0u8; 12];
        file.read_exact(&mut buffer)?;

        let sig = u32::from_le_bytes(buffer[0..4].try_into().unwrap());
        if sig != VPK_SIGNATURE {
            return Err(Source2Error::Vpk("bad VPK signature".into()));
        }

        let version = u32::from_le_bytes(buffer[4..8].try_into().unwrap());
        if version != 1 && version != 2 {
            return Err(Source2Error::Vpk(format!(
                "unsupported VPK version {version}"
            )));
        }
        let tree_length = u32::from_le_bytes(buffer[8..12].try_into().unwrap()) as usize;
        let tree_start: usize = if version >= 2 { 28 } else { 12 };
        let data_section_start = tree_start + tree_length;
        let inline_data_length = if version == 2 {
            let mut rest = [0; 16];
            file.read_exact(&mut rest)?;
            Some(u32::from_le_bytes(rest[..4].try_into().unwrap()))
        } else {
            None
        };

        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(tree_start as u64))?;
        let mut tree = vec![0u8; tree_length];
        file.read_exact(&mut tree)?;
        let tree = tree.as_slice();
        let entries = parse_tree(tree)?;
        let mut entry_index = HashMap::new();
        for (index, entry) in entries.iter().enumerate() {
            entry_index
                .entry(entry.full_path.to_ascii_lowercase())
                .or_insert(index);
        }

        Ok(Self {
            path: vpk_path.to_path_buf(),
            companion_dir: companion_dir(vpk_path),
            data_section_start,
            inline_data_length,
            entries,
            entry_index,
            overlay_dir: None,
            fallbacks: Vec::new(),
            evidence: None,
        })
    }

    /// Open the archive with an unpacked tree shadowing it (see `overlay_dir`).
    pub fn open_with_overlay(vpk_path: &Path, overlay_dir: Option<&Path>) -> Result<Self> {
        let mut archive = Self::open(vpk_path)?;
        archive.overlay_dir = overlay_dir
            .filter(|dir| dir.is_dir())
            .map(Path::to_path_buf);
        Ok(archive)
    }

    /// The overlay's path for an entry, if the overlay holds one. Entry paths
    /// come from the VPK directory, so they are already relative and separator-
    /// normalized, but a `..` is rejected rather than trusted.
    fn overlay_path(&self, entry_path: &str) -> Option<PathBuf> {
        let overlay_dir = self.overlay_dir.as_ref()?;
        let path = overlay_dir.join(sanitize_relative(entry_path)?);
        path.is_file().then_some(path)
    }

    /// Read missing resources from lower-priority archives without unpacking them.
    pub fn with_fallbacks(mut self, fallbacks: Vec<Arc<VpkArchive>>) -> Self {
        self.fallbacks = fallbacks;
        self
    }

    /// Enable evidence only for this request; shared cached base archives remain immutable.
    pub fn with_preview_report(mut self) -> Self {
        self.evidence = Some(Mutex::new(PreviewEvidence::default()));
        self
    }

    pub fn preview_report(&self) -> PreviewReport {
        crate::preview_report::snapshot(&self.evidence)
    }

    /// Return the actual byte source without reading it. A present corrupt override still wins.
    pub fn entry_source(&self, entry_path: &str) -> Option<PreviewDependency> {
        let key = entry_path.replace('\\', "/").to_ascii_lowercase();
        let overlay = self.overlay_path(entry_path).is_some();
        if overlay || self.entry_index.contains_key(&key) {
            return Some(PreviewDependency {
                path: key,
                archive: self.path.to_string_lossy().into_owned(),
                overlay,
            });
        }
        self.fallback_for(entry_path)?.entry_source(entry_path)
    }

    pub fn record_preview_issue(
        &self,
        path: &str,
        kind: PreviewIssueKind,
        detail: impl Into<String>,
    ) {
        if let Some(evidence) = &self.evidence {
            evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .issue(PreviewIssue {
                    path: path.to_string(),
                    kind,
                    detail: detail.into(),
                });
        }
    }

    pub fn record_preview_error(&self, path: &str, error: &Source2Error) {
        let kind = match error {
            Source2Error::EntryNotFound(_) => PreviewIssueKind::MissingDependency,
            Source2Error::UnsupportedFormat(_) => PreviewIssueKind::Unsupported,
            _ => match self.entry_source(path) {
                Some(source) if source.overlay || source.archive == self.path.to_string_lossy() => {
                    PreviewIssueKind::CorruptOverride
                }
                _ => PreviewIssueKind::InvalidGameAsset,
            },
        };
        self.record_preview_issue(path, kind, error.to_string());
    }

    fn record_read(&self, path: &str, result: &Result<Vec<u8>>) {
        if self.evidence.is_none() {
            return;
        }
        if let Some(source) = self.entry_source(path)
            && let Some(evidence) = &self.evidence
        {
            evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .dependency(source);
        }
        if let Err(error) = result {
            self.record_preview_error(path, error);
        }
    }

    pub fn list_entries(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.full_path.clone())
            .collect()
    }

    pub fn contains_entry(&self, entry_path: &str) -> bool {
        self.overlay_path(entry_path).is_some()
            || self
                .entry_index
                .contains_key(&entry_path.replace('\\', "/").to_ascii_lowercase())
            || self.fallback_for(entry_path).is_some()
    }

    fn fallback_for(&self, entry_path: &str) -> Option<&VpkArchive> {
        self.fallbacks
            .iter()
            .find(|archive| archive.contains_entry(entry_path))
            .map(Arc::as_ref)
    }

    /// Whether an entry stores a prefix in the directory tree. Extraction includes this prefix.
    pub fn has_preload_bytes(&self, entry_path: &str) -> bool {
        let key = entry_path.replace('\\', "/").to_ascii_lowercase();
        self.entry_index
            .get(&key)
            .and_then(|index| self.entries.get(*index))
            .is_some_and(|entry| !entry.preload.is_empty())
    }

    pub fn extract_entry(&self, entry_path: &str) -> Result<Vec<u8>> {
        let result = self.extract_entry_inner(entry_path);
        self.record_read(entry_path, &result);
        result
    }

    fn extract_entry_inner(&self, entry_path: &str) -> Result<Vec<u8>> {
        if let Some(overlay) = self.overlay_path(entry_path) {
            return Ok(fs::read(overlay)?);
        }
        let key = entry_path.replace('\\', "/").to_ascii_lowercase();
        let Some(entry) = self
            .entry_index
            .get(&key)
            .and_then(|index| self.entries.get(*index))
        else {
            return match self.fallback_for(entry_path) {
                Some(archive) => archive.extract_entry(entry_path),
                None => Err(Source2Error::EntryNotFound(entry_path.to_string())),
            };
        };
        self.read_entry(entry)
    }

    /// Reject oversized preview assets before allocating their archive payload.
    pub fn extract_entry_limited(&self, entry_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let result = self.extract_entry_limited_inner(entry_path, max_bytes);
        self.record_read(entry_path, &result);
        result
    }

    fn extract_entry_limited_inner(&self, entry_path: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let too_large =
            || Source2Error::Vpk(format!("entry exceeds preview byte limit: {entry_path}"));
        if let Some(overlay) = self.overlay_path(entry_path) {
            let file = fs::File::open(overlay)?;
            if file.metadata()?.len() > max_bytes as u64 {
                return Err(too_large());
            }
            let mut bytes = Vec::new();
            file.take(max_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes)?;
            if bytes.len() > max_bytes {
                return Err(too_large());
            }
            return Ok(bytes);
        }
        let key = entry_path.replace('\\', "/").to_ascii_lowercase();
        if let Some(entry) = self
            .entry_index
            .get(&key)
            .and_then(|index| self.entries.get(*index))
        {
            if entry.preload.len() as u64 + u64::from(entry.entry_length) > max_bytes as u64 {
                return Err(too_large());
            }
            return self.read_entry(entry);
        }
        match self.fallback_for(entry_path) {
            Some(archive) => archive.extract_entry_limited(entry_path, max_bytes),
            None => Err(Source2Error::EntryNotFound(entry_path.to_string())),
        }
    }

    /// Read the bytes of one already-resolved entry, pulling from inline data or
    /// the matching `_NNN.vpk` companion archive.
    fn read_entry(&self, entry: &EntryInfo) -> Result<Vec<u8>> {
        let mut bytes = entry.preload.clone();
        if entry.entry_length == 0 {
            return Ok(bytes);
        }

        if entry.archive_index == 0x7fff {
            if self.inline_data_length.is_some_and(|length| {
                u64::from(entry.entry_offset) + u64::from(entry.entry_length) > u64::from(length)
            }) {
                return Err(Source2Error::Vpk(
                    "inline entry exceeds the VPK data section".into(),
                ));
            }
            let start = self.data_section_start as u64 + u64::from(entry.entry_offset);
            let mut file = fs::File::open(&self.path)?;
            if start + u64::from(entry.entry_length) > file.metadata()?.len() {
                return Err(Source2Error::Vpk(
                    "inline entry exceeds the VPK file".into(),
                ));
            }
            file.seek(SeekFrom::Start(start))?;
            let prefix = bytes.len();
            bytes.resize(prefix + entry.entry_length as usize, 0);
            file.read_exact(&mut bytes[prefix..])?;
            return Ok(bytes);
        }

        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let base = stem.strip_suffix("_dir").unwrap_or(stem);
        let archive_name = format!("{base}_{:03}.vpk", entry.archive_index);
        let archive_path = self.companion_dir.join(&archive_name);
        let mut archive_file = fs::File::open(&archive_path)?;
        if u64::from(entry.entry_offset) + u64::from(entry.entry_length)
            > archive_file.metadata()?.len()
        {
            return Err(Source2Error::Vpk(
                "entry exceeds its companion archive".into(),
            ));
        }
        archive_file.seek(SeekFrom::Start(u64::from(entry.entry_offset)))?;
        let prefix = bytes.len();
        bytes.resize(prefix + entry.entry_length as usize, 0);
        archive_file.read_exact(&mut bytes[prefix..])?;
        Ok(bytes)
    }

    /// Unpack every entry to `dest_dir`, preserving the VPK's internal directory
    /// layout. Returns the number of files written. Entries whose path would
    /// escape `dest_dir` (contain `..`, absolute/root components) are skipped.
    /// This only reads the source VPK; nothing is moved or removed.
    pub fn extract_all_to(&self, dest_dir: &Path) -> Result<usize> {
        let mut written = 0usize;
        for entry in &self.entries {
            written += self.write_entry(entry, dest_dir)?;
        }
        Ok(written)
    }

    /// Unpack only the named entries to `dest_dir` (case-insensitive match),
    /// preserving their internal directory layout. Used to pull a single hero's
    /// assets out of the shared base-game pak without unpacking gigabytes.
    pub fn extract_entries_to(&self, dest_dir: &Path, entry_paths: &[String]) -> Result<usize> {
        let mut written = 0usize;
        for entry_path in entry_paths {
            let key = entry_path.replace('\\', "/").to_ascii_lowercase();
            if let Some(entry) = self
                .entry_index
                .get(&key)
                .and_then(|index| self.entries.get(*index))
            {
                written += self.write_entry(entry, dest_dir)?;
            }
        }
        Ok(written)
    }

    /// Write one entry's bytes under `dest_dir` at its sanitized relative path.
    /// Returns 1 if written, 0 if the path was rejected as unsafe.
    fn write_entry(&self, entry: &EntryInfo, dest_dir: &Path) -> Result<usize> {
        let Some(rel) = sanitize_relative(&entry.full_path) else {
            return Ok(0);
        };
        let out_path = dest_dir.join(&rel);
        let data = self.read_entry(entry)?;
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out_path, &data)?;
        Ok(1)
    }
}

/// Normalize resource separators, rejecting empty, dot, parent, drive, stream,
/// control-character, Windows device-name and trailing-dot/space segments on every platform.
pub fn normalize_resource_path(entry_path: &str) -> Option<String> {
    let normalized = entry_path.replace('\\', "/");
    for segment in normalized.split('/') {
        let stem = segment
            .split('.')
            .next()
            .unwrap_or(segment)
            .trim_end_matches(' ');
        let device = stem.eq_ignore_ascii_case("con")
            || stem.eq_ignore_ascii_case("prn")
            || stem.eq_ignore_ascii_case("aux")
            || stem.eq_ignore_ascii_case("nul")
            // Windows also reserves the superscript digits ¹ ² ³ after COM and LPT.
            || stem
                .get(..3)
                .zip(stem.get(3..))
                .is_some_and(|(prefix, digit)| {
                    (prefix.eq_ignore_ascii_case("com") || prefix.eq_ignore_ascii_case("lpt"))
                        && matches!(
                            digit,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                });
        if matches!(segment, "" | "." | "..")
            || segment.ends_with(['.', ' '])
            || segment.contains(':')
            || segment.chars().any(char::is_control)
            || device
        {
            return None;
        }
    }
    Some(normalized)
}

fn sanitize_relative(entry_path: &str) -> Option<PathBuf> {
    normalize_resource_path(entry_path).map(PathBuf::from)
}

/// Read the raw bytes of a single entry inside a VPK, resolving inline data and
/// companion `_NNN.vpk` archives. Mirrors the game's VPK v2 layout.
pub fn extract_entry(vpk_path: &Path, entry_path: &str) -> Result<Vec<u8>> {
    VpkArchive::open(vpk_path)?.extract_entry(entry_path)
}

/// List every entry path in a VPK (used to resolve the actual texture path when
/// the caller only knows a filename).
pub fn list_entries(vpk_path: &Path) -> Result<Vec<String>> {
    Ok(VpkArchive::open(vpk_path)?.list_entries())
}

/// Unpack an entire VPK into `dest_dir`, preserving its directory layout.
/// Read-only with respect to the source VPK. Returns the file count written.
pub fn extract_all(vpk_path: &Path, dest_dir: &Path) -> Result<usize> {
    VpkArchive::open(vpk_path)?.extract_all_to(dest_dir)
}

/// Unpack only the named entries from a VPK into `dest_dir`. Read-only with
/// respect to the source VPK. Returns the file count written.
pub fn extract_entries(vpk_path: &Path, dest_dir: &Path, entry_paths: &[String]) -> Result<usize> {
    VpkArchive::open(vpk_path)?.extract_entries_to(dest_dir, entry_paths)
}


fn parse_tree(tree: &[u8]) -> Result<Vec<EntryInfo>> {
    fn cstr<'a>(tree: &'a [u8], at: &mut usize) -> Result<&'a str> {
        let rest = tree
            .get(*at..)
            .ok_or_else(|| Source2Error::Vpk("VPK tree is cut short".into()))?;
        let end = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Source2Error::Vpk("VPK tree is cut short".into()))?;
        *at += end + 1;
        std::str::from_utf8(&rest[..end]).map_err(|_| Source2Error::Vpk("VPK path is not text".into()))
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let ext = cstr(tree, &mut at)?;
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = cstr(tree, &mut at)?;
            if dir.is_empty() {
                break;
            }
            loop {
                let name = cstr(tree, &mut at)?;
                if name.is_empty() {
                    break;
                }
                let rec = tree
                    .get(at..at + 18)
                    .ok_or_else(|| Source2Error::Vpk("VPK tree is cut short".into()))?;
                let preload_len = u16::from_le_bytes([rec[4], rec[5]]) as usize;
                let archive_index = u16::from_le_bytes([rec[6], rec[7]]);
                let entry_offset = u32::from_le_bytes(rec[8..12].try_into().unwrap());
                let entry_length = u32::from_le_bytes(rec[12..16].try_into().unwrap());
                at += 18;
                let preload = tree
                    .get(at..at + preload_len)
                    .ok_or_else(|| Source2Error::Vpk("VPK tree is cut short".into()))?
                    .to_vec();
                at += preload_len;
                let full_path = if dir == " " {
                    format!("{name}.{ext}")
                } else {
                    format!("{dir}/{name}.{ext}")
                };
                out.push(EntryInfo {
                    full_path,
                    archive_index,
                    entry_offset,
                    entry_length,
                    preload,
                });
            }
        }
    }
    Ok(out)
}
