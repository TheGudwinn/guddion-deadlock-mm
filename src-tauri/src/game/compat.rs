//! Compatibility scan of the installed mods. Read-only: nothing in the game or in the mods is changed.
//!
//! What it looks at, per enabled mod:
//! - hero models that have no camera interface (`AttachmentCameraData`) in their data. Such a model
//!   can give a broken or stuck camera on the current game version;
//! - hero models that two or more enabled mods both replace (only one of them wins in the game);
//! - models this reader could not look into (reported in the scan details, never as a problem).
//!
//! A finding is a hint, not a proof of a defect; the page says so.
use super::resource::{contains, data_block, key_values, kv3_strings, replace_data_block, with_key_values};
use crate::install::vpk::{read_entry, read_vpk_entries};
use crate::library::scan::ExistingAddon;
use serde::Serialize;
use std::collections::BTreeMap;

const CAMERA_MARK: &[u8] = b"AttachmentCameraData";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompatFinding {
    /// `camera_interface_missing` | `model_overlap`
    pub kind: String,
    pub severity: &'static str,
    pub model: String,
    /// Mod files involved (names as in the addons folder).
    pub mods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ScanDetails {
    pub mods_scanned: usize,
    pub mods_unreadable: Vec<String>,
    pub hero_models: usize,
    pub models_looked_into: usize,
    /// (mod file, model, reason) for models this reader cannot look into.
    pub skipped: Vec<(String, String, String)>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CompatReport {
    pub findings: Vec<CompatFinding>,
    pub details: ScanDetails,
}

/// `models/heroes_staging/mirage_v2/mirage.vmdl_c` → true. Other models of a hero (props, effects,
/// backups, nested folders) are not "the hero model" and are left alone.
pub fn is_hero_model(path: &str) -> bool {
    let p: Vec<&str> = path.split('/').collect();
    if p.len() != 4 || p[0] != "models" || !p[1].starts_with("heroes") {
        return false;
    }
    let Some(stem) = p[3].strip_suffix(".vmdl_c") else { return false };
    stem.len() >= 3 && p[2].starts_with(stem) && (p[2].len() == stem.len() || p[2][stem.len()..].starts_with('_'))
}

pub fn scan(addons: &[ExistingAddon]) -> CompatReport {
    let mut rep = CompatReport::default();
    // model path -> mods (in the order scanned) that contain it
    let mut by_model: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut no_camera: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut repaired: std::collections::BTreeSet<String> = Default::default();

    for a in addons.iter().filter(|a| !a.disabled) {
        let data = match read_vpk_entries(&a.path) {
            Ok(d) => d,
            Err(_) => {
                rep.details.mods_unreadable.push(a.file_name.clone());
                continue;
            }
        };
        rep.details.mods_scanned += 1;
        if a.file_name == PACKAGE_FILE {
            // our own repairs: the models in it count as repaired, they are not "mods" to check
            repaired.extend(data.entries.iter().filter(|e| is_hero_model(&e.path)).map(|e| e.path.clone()));
            continue;
        }
        for e in data.entries.iter().filter(|e| is_hero_model(&e.path)) {
            rep.details.hero_models += 1;
            by_model.entry(e.path.clone()).or_default().push(a.file_name.clone());
            let skip = |rep: &mut CompatReport, why: String| rep.details.skipped.push((a.file_name.clone(), e.path.clone(), why));
            let bytes = match read_entry(&a.path, &data, e) {
                Ok(b) => b,
                Err(why) => {
                    skip(&mut rep, why);
                    continue;
                }
            };
            let Some(block) = data_block(&bytes) else {
                skip(&mut rep, "no data block".into());
                continue;
            };
            match kv3_strings(block) {
                Ok(strings) => {
                    rep.details.models_looked_into += 1;
                    if !contains(&strings, CAMERA_MARK) {
                        no_camera.entry(e.path.clone()).or_default().push(a.file_name.clone());
                    }
                }
                Err(why) => skip(&mut rep, why.to_string()),
            }
        }
    }

    for (model, mods) in no_camera.into_iter().filter(|(m, _)| !repaired.contains(m)) {
        rep.findings.push(CompatFinding { kind: "camera_interface_missing".into(), severity: "warn", model, mods });
    }
    for (model, mods) in by_model {
        if mods.len() > 1 {
            rep.findings.push(CompatFinding { kind: "model_overlap".into(), severity: "info", model, mods });
        }
    }
    rep
}

/// Our compatibility package. It starts with `00` and uses slot `pak00`, so it sorts before every mod
/// and before the lowest slot. UNVERIFIED in the game, like the slot order itself (see `pak_order`).
pub const PACKAGE_FILE: &str = "00compat_pak00_dir.vpk";

/// The `AttachmentCameraData` block of a model's key-values text, with its indentation and final newline.
pub fn camera_block(kv: &str) -> Option<String> {
    let start = kv.find("\n\tAttachmentCameraData =")? + 1;
    let open = start + kv[start..].find('[')?;
    let (mut depth, mut in_str) = (0i32, false);
    for (i, c) in kv[open..].char_indices() {
        match c {
            '"' => in_str = !in_str,
            '[' if !in_str => depth += 1,
            ']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    let mut end = open + i + 1;
                    if kv[end..].starts_with('\n') {
                        end += 1;
                    }
                    let mut b = kv[start..end].to_string();
                    if !b.ends_with('\n') {
                        b.push('\n');
                    }
                    return Some(b);
                }
            }
            _ => {}
        }
    }
    None
}

/// `kv` with `block` added at the end of its top-level section. `None` when the text already has one
/// or has no closing brace.
pub fn with_camera_block(kv: &str, block: &str) -> Option<String> {
    if kv.contains("AttachmentCameraData") {
        return None;
    }
    let close = kv.rfind('}')?;
    let before = &kv[..close];
    let mut out = String::with_capacity(kv.len() + block.len() + 1);
    out.push_str(before);
    if !before.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    out.push_str(&kv[close..]);
    Some(out)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RepairReport {
    pub fixed: Vec<String>,
    /// (model, why it was left alone)
    pub skipped: Vec<(String, String)>,
}

/// Load-slot number in a file name like `pak34_dir.vpk` or `625513_pak05_dir.vpk`; lower wins by assumption.
fn slot(file_name: &str) -> u32 {
    file_name.split("_pak").nth(1).or_else(|| file_name.strip_prefix("pak")).and_then(|r| r.get(..2)).and_then(|d| d.parse().ok()).unwrap_or(99)
}

/// Builds the new content of the compatibility package: models of the current package that are not
/// being redone, plus the repaired ones. Nothing is written here.
pub fn repair_cameras(game_pak: &std::path::Path, addons: &[ExistingAddon], package: &std::path::Path) -> Result<(Vec<(String, Vec<u8>)>, RepairReport), String> {
    let found = scan(addons);
    let mut rep = RepairReport::default();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let targets: Vec<&CompatFinding> = found.findings.iter().filter(|f| f.kind == "camera_interface_missing").collect();
    if targets.is_empty() {
        return Ok((files, rep));
    }
    let vanilla = read_vpk_entries(game_pak).map_err(|e| format!("game files: {e}"))?;
    for f in targets {
        let skip = |rep: &mut RepairReport, why: &str| rep.skipped.push((f.model.clone(), why.to_string()));
        // the mod that wins in the game is the one with the lowest slot
        let Some(winner) = f.mods.iter().min_by_key(|m| slot(m)) else { continue };
        let Some(addon) = addons.iter().find(|a| &a.file_name == winner) else { continue };
        let Some(v_entry) = vanilla.entries.iter().find(|e| e.path == f.model) else {
            skip(&mut rep, "the game has no model at this path");
            continue;
        };
        let Ok(v_bytes) = read_entry(game_pak, &vanilla, v_entry) else {
            skip(&mut rep, "cannot read the game's model");
            continue;
        };
        let v_kv = match data_block(&v_bytes).ok_or("no data block".to_string()).and_then(key_values) {
            Ok(k) => k,
            Err(why) => {
                skip(&mut rep, &format!("game model: {why}"));
                continue;
            }
        };
        let Some(block) = camera_block(&v_kv) else {
            skip(&mut rep, "the game's model has no camera data either");
            continue;
        };
        let data = read_vpk_entries(&addon.path)?;
        let Some(m_entry) = data.entries.iter().find(|e| e.path == f.model) else { continue };
        let m_bytes = match read_entry(&addon.path, &data, m_entry) {
            Ok(b) => b,
            Err(why) => {
                skip(&mut rep, &why);
                continue;
            }
        };
        let patched = (|| -> Result<Vec<u8>, String> {
            let blk = data_block(&m_bytes).ok_or("no data block")?;
            let kv = key_values(blk)?;
            let new_kv = with_camera_block(&kv, &block).ok_or("text cannot take the camera data")?;
            let new_blk = with_key_values(blk, &new_kv)?;
            let out = replace_data_block(&m_bytes, &new_blk)?;
            // read it back the way the game will
            let again = data_block(&out).ok_or("no data block after the change")?;
            if !key_values(again)?.contains("AttachmentCameraData") || kv3_strings(again).map_err(|e| e.to_string())?.is_empty() {
                return Err("the result does not read back".into());
            }
            Ok(out)
        })();
        match patched {
            Ok(bytes) => {
                files.push((f.model.clone(), bytes));
                rep.fixed.push(f.model.clone());
            }
            Err(why) => skip(&mut rep, &why),
        }
    }
    // keep what an earlier repair put into the package
    if package.exists() {
        if let Ok(old) = read_vpk_entries(package) {
            for e in &old.entries {
                if !files.iter().any(|(p, _)| p == &e.path) {
                    if let Ok(b) = read_entry(package, &old, e) {
                        files.push((e.path.clone(), b));
                    }
                }
            }
        }
    }
    Ok((files, rep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn camera_block_is_cut_out_and_put_back() {
        let vanilla = "<!-- kv3 -->\n{\n\tA = 1\n\tAttachmentCameraData = \n\t[\n\t\t{\n\t\t\tattachment_name = \"near_00\"\n\t\t},\n\t]\n\tB = 2\n}\n";
        let block = camera_block(vanilla).unwrap();
        assert!(block.starts_with("\tAttachmentCameraData ="));
        assert!(block.ends_with("\t]\n"));
        assert!(!block.contains("B = 2"));
        let old = "<!-- kv3 -->\n{\n\tC = 3\n}\n";
        let new = with_camera_block(old, &block).unwrap();
        assert!(new.contains("\tC = 3\n\tAttachmentCameraData"));
        assert!(new.ends_with("\t]\n}\n"));
        assert!(with_camera_block(&new, &block).is_none());
        assert_eq!(camera_block(old), None);
    }

    #[test]
    fn slots_are_read_from_names() {
        assert_eq!(slot("pak34_dir.vpk"), 34);
        assert_eq!(slot("625513_pak05_dir.vpk"), 5);
        assert_eq!(slot("local-x_dir.vpk"), 99);
    }

    #[test]
    fn hero_models_are_told_from_the_rest() {
        assert!(is_hero_model("models/heroes_staging/mirage_v2/mirage.vmdl_c"));
        assert!(is_hero_model("models/heroes_wip/wraith/wraith.vmdl_c"));
        assert!(!is_hero_model("models/heroes_staging/bebop/bebop_backup.vmdl_c"));
        assert!(!is_hero_model("models/heroes_staging/mirage_v2/shop_hill.vmdl_c"));
        assert!(!is_hero_model("models/heroes_staging/haze/lucy/lucy_select.vmdl_c"));
        assert!(!is_hero_model("models/props/chest.vmdl_c"));
        assert!(!is_hero_model("models/heroes_wip/wraith/wraith.vmat_c"));
    }

    #[test]
    fn unreadable_models_are_not_findings() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("pak01_dir.vpk");
        std::fs::write(&p, crate::install::vpk::build_vpk(&[("models/heroes_wip/wraith/wraith.vmdl_c", b"not a resource")])).unwrap();
        let addon = ExistingAddon { file_name: "pak01_dir.vpk".into(), path: p, ours: false, disabled: false, size: 0, entries: Ok(vec![]) };
        let r = scan(&[addon]);
        assert!(r.findings.is_empty());
        assert_eq!(r.details.hero_models, 1);
        assert_eq!(r.details.skipped.len(), 1);
    }

    /// Repairs the real mods into a package under `REPAIR_OUT`; the game folder is only read.
    #[test]
    fn real_repair() {
        let (Ok(dir), Ok(out)) = (std::env::var("DEADLOCK_ADDONS"), std::env::var("REPAIR_OUT")) else { return };
        let addons_dir = PathBuf::from(dir);
        let game_pak = addons_dir.parent().unwrap().join("pak01_dir.vpk");
        let addons = crate::library::scan::scan_addons(&addons_dir).unwrap();
        let pkg = PathBuf::from(&out).join(PACKAGE_FILE);
        let (files, rep) = repair_cameras(&game_pak, &addons, &pkg).unwrap();
        eprintln!("fixed {:?}\nskipped {:?}", rep.fixed, rep.skipped);
        assert!(!files.is_empty());
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        std::fs::write(&pkg, crate::install::vpk::build_vpk(&refs)).unwrap();
    }

    /// Against real mods: `DEADLOCK_ADDONS=<addons folder> cargo test … real_addons -- --nocapture`.
    #[test]
    fn real_addons() {
        let Ok(dir) = std::env::var("DEADLOCK_ADDONS") else { return };
        let addons = crate::library::scan::scan_addons(&PathBuf::from(dir)).unwrap();
        let r = scan(&addons);
        eprintln!("{:?}", r.details.mods_scanned);
        eprintln!("hero models {} looked into {}", r.details.hero_models, r.details.models_looked_into);
        for f in &r.findings {
            eprintln!("{} {} {:?}", f.kind, f.model, f.mods);
        }
        for s in &r.details.skipped {
            eprintln!("skipped {s:?}");
        }
        assert!(r.details.models_looked_into > 0);
    }
}
