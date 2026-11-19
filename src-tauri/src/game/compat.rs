//! Compatibility scan and camera repair for the installed hero mods.
//!
//! The scan is read-only. For every enabled hero model it compares the mod with the game's own model at
//! the same path and looks at two things the camera needs:
//! - the camera list in the model's key-values text (`AttachmentCameraData`): missing or not the list
//!   the current game model has;
//! - the axes the camera points on the root bone are written in (see `rig`): in the turned axes the
//!   camera does not follow the hero.
//!
//! It also reports hero models several enabled mods replace, and models this reader cannot look into
//! (scan details only). A finding is a hint, not proof of a defect; the page says so.
use super::kv3::Kv3;
use super::resource::{block_table, contains, data_block, key_values, kv3_strings, replace_block_at, with_key_values};
use super::rig::{self, Basis};
use crate::install::vpk::{read_entry, read_vpk_entries, VpkData};
use crate::library::scan::ExistingAddon;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const CAMERA_MARK: &[u8] = b"AttachmentCameraData";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CompatFinding {
    /// `camera_interface_missing` | `camera_interface_outdated` | `camera_basis_rotated` | `model_overlap` | `package_inactive`
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

/// A mod whose hero models needed the camera repair and are now covered by the compatibility package.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FixedMod {
    /// File name as in the addons folder.
    pub file: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CompatReport {
    pub findings: Vec<CompatFinding>,
    /// Mods that were repaired; they are not findings any more.
    pub fixed: Vec<FixedMod>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cam {
    Ok,
    Missing,
    Outdated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    pub cam: Cam,
    pub basis: Basis,
    /// Why the camera points could not be compared, when they could not.
    pub note: Option<String>,
}

impl Analysis {
    pub fn needs_repair(&self) -> bool {
        self.cam != Cam::Ok || self.basis == Basis::Rotated
    }
}

/// Names listed by a camera block.
fn camera_names(block: &str) -> BTreeSet<String> {
    block.split("attachment_name = \"").skip(1).filter_map(|r| r.split('"').next()).map(str::to_string).collect()
}

fn kv_of(res: &[u8]) -> Result<String, String> {
    key_values(data_block(res).ok_or("no data block")?)
}

fn first_rig(res: &[u8]) -> Result<rig::Rig, String> {
    let t = block_table(res).ok_or("resource header")?;
    let (_, start, size) = *t.iter().find(|b| b.0 == *b"MDAT").ok_or("no mesh data")?;
    rig::read_rig(&Kv3::open(&res[start..start + size])?)
}

/// Compares a model with the game's model at the same path.
pub fn analyze(model: &[u8], game: &[u8]) -> Result<Analysis, String> {
    let game_block = camera_block(&kv_of(game)?);
    let cam = match (camera_block(&kv_of(model)?), &game_block) {
        (_, None) => Cam::Ok,
        (None, Some(_)) => Cam::Missing,
        (Some(m), Some(g)) => {
            if camera_names(&m) == camera_names(g) {
                Cam::Ok
            } else {
                Cam::Outdated
            }
        }
    };
    let (basis, note) = match (first_rig(model), first_rig(game)) {
        (Ok(m), Ok(g)) => (rig::basis(&m, &g), None),
        (Err(e), _) => (Basis::Unknown, Some(format!("mesh data: {e}"))),
        (_, Err(e)) => (Basis::Unknown, Some(format!("game mesh data: {e}"))),
    };
    Ok(Analysis { cam, basis, note })
}

/// Opens the game's `pak01_dir.vpk` for comparisons.
pub struct GameModels<'a> {
    path: &'a Path,
    data: VpkData,
}

impl<'a> GameModels<'a> {
    pub fn open(path: &'a Path) -> Result<Self, String> {
        Ok(GameModels { path, data: read_vpk_entries(path).map_err(|e| format!("game files: {e}"))? })
    }
    pub fn model(&self, p: &str) -> Option<Vec<u8>> {
        let e = self.data.entries.iter().find(|e| e.path == p)?;
        read_entry(self.path, &self.data, e).ok()
    }
}

pub fn scan(addons: &[ExistingAddon], package: Option<&Path>, game_pak: Option<&Path>) -> CompatReport {
    let mut rep = CompatReport::default();
    let game = game_pak.and_then(|p| GameModels::open(p).ok());
    // model path -> mods (in the order scanned) that contain it
    let mut by_model: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut found: BTreeMap<(String, &'static str), Vec<String>> = BTreeMap::new();
    let mut repaired: BTreeSet<String> = Default::default();

    // our own repairs: the models in the package count as repaired
    if let Some(p) = package.filter(|p| p.exists()) {
        if let Ok(d) = read_vpk_entries(p) {
            for e in d.entries.iter().filter(|e| is_hero_model(&e.path)) {
                // a model counts as repaired only when it checks out against the game's model
                let good = match (game.as_ref().and_then(|g| g.model(&e.path)), read_entry(p, &d, e)) {
                    (Some(g), Ok(m)) => analyze(&m, &g).map(|a| !a.needs_repair()).unwrap_or(false),
                    (None, Ok(_)) => true,
                    _ => false,
                };
                if good {
                    repaired.insert(e.path.clone());
                }
            }
        }
    }
    for a in addons.iter().filter(|a| !a.disabled && a.file_name != LEGACY_PACKAGE) {
        let data = match read_vpk_entries(&a.path) {
            Ok(d) => d,
            Err(_) => {
                rep.details.mods_unreadable.push(a.file_name.clone());
                continue;
            }
        };
        rep.details.mods_scanned += 1;
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
            let strings = match kv3_strings(block) {
                Ok(s) => s,
                Err(why) => {
                    skip(&mut rep, why.to_string());
                    continue;
                }
            };
            rep.details.models_looked_into += 1;
            // against the game's own model when it is available
            if let Some(g) = game.as_ref().and_then(|g| g.model(&e.path)) {
                match analyze(&bytes, &g) {
                    Ok(an) => {
                        if let Some(n) = &an.note {
                            rep.details.skipped.push((a.file_name.clone(), e.path.clone(), n.clone()));
                        }
                        match an.cam {
                            Cam::Missing => found.entry((e.path.clone(), "camera_interface_missing")).or_default().push(a.file_name.clone()),
                            Cam::Outdated => found.entry((e.path.clone(), "camera_interface_outdated")).or_default().push(a.file_name.clone()),
                            Cam::Ok => {}
                        }
                        if an.basis == Basis::Rotated {
                            found.entry((e.path.clone(), "camera_basis_rotated")).or_default().push(a.file_name.clone());
                        }
                    }
                    Err(why) => skip(&mut rep, why),
                }
            } else if !contains(&strings, CAMERA_MARK) {
                found.entry((e.path.clone(), "camera_interface_missing")).or_default().push(a.file_name.clone());
            }
        }
    }

    let mut fixed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((model, kind), mods) in found {
        if repaired.contains(&model) {
            for m in mods {
                fixed.entry(m).or_default().insert(model.clone());
            }
        } else {
            rep.findings.push(CompatFinding { kind: kind.into(), severity: "warn", model, mods });
        }
    }
    rep.fixed = fixed.into_iter().map(|(file, models)| FixedMod { file, models: models.into_iter().collect() }).collect();
    for (model, mods) in by_model {
        if mods.len() > 1 {
            rep.findings.push(CompatFinding { kind: "model_overlap".into(), severity: "info", model, mods });
        }
    }
    rep
}

/// Our compatibility package lives in its own folder next to `addons`, listed in `gameinfo.gi` before
/// `citadel/addons`: an earlier search path wins, so it beats every mod whatever the mod's slot is.
pub const COMPAT_DIR: &str = "guddion_compat";
pub const PACKAGE_FILE: &str = "pak01_dir.vpk";
/// The first version put the package into `addons`; it is moved out when a repair runs.
pub const LEGACY_PACKAGE: &str = "00compat_pak00_dir.vpk";
/// Search path as written in `gameinfo.gi`.
pub const SEARCH_PATH: &str = "citadel/guddion_compat";

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

/// `kv` with the camera block of the game's model: the model's own block is replaced, or added at the
/// end of the top-level section when it has none.
pub fn with_game_camera(kv: &str, game_block: &str) -> Option<String> {
    match camera_block(kv) {
        Some(old) => Some(kv.replacen(&old, game_block, 1)),
        None => with_camera_block(kv, game_block),
    }
}

/// The model with the camera list of the game's model and, when its points are written in the root
/// bone's turned axes, with every root point turned back into model axes. Read back and checked.
pub fn patch_model(model: &[u8], game: &[u8], an: &Analysis) -> Result<Vec<u8>, String> {
    let mut out = model.to_vec();
    if an.basis == Basis::Rotated {
        let t = block_table(&out).ok_or("resource header")?;
        let mdats: Vec<usize> = t.iter().enumerate().filter(|(_, b)| b.0 == *b"MDAT").map(|(i, _)| i).collect();
        if mdats.is_empty() {
            return Err("no mesh data".into());
        }
        for i in mdats {
            let (_, start, size) = block_table(&out).ok_or("resource header")?[i];
            let mut k = Kv3::open(&out[start..start + size])?;
            let rg = rig::read_rig(&k)?;
            rig::convert_to_model_axes(&mut k, &rg);
            out = replace_block_at(&out, i, &k.finish())?;
        }
    }
    if an.cam != Cam::Ok {
        let game_block = camera_block(&kv_of(game)?).ok_or("the game's model has no camera data")?;
        let blk = data_block(&out).ok_or("no data block")?;
        let new_kv = with_game_camera(&key_values(blk)?, &game_block).ok_or("text cannot take the camera data")?;
        let new_blk = with_key_values(blk, &new_kv)?;
        let idx = block_table(&out).and_then(|t| t.iter().rposition(|b| b.0 == *b"DATA")).ok_or("no data block")?;
        out = replace_block_at(&out, idx, &new_blk)?;
    }
    // read it back the way the game will
    let again = analyze(&out, game)?;
    if again.cam != Cam::Ok || again.basis == Basis::Rotated {
        return Err(format!("the result does not read back ({again:?})"));
    }
    Ok(out)
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
pub fn repair_cameras(game_pak: &Path, addons: &[ExistingAddon], package: &Path, legacy: &Path) -> Result<(Vec<(String, Vec<u8>)>, RepairReport), String> {
    let found = scan(addons, Some(package), Some(game_pak));
    let mut rep = RepairReport::default();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    // one entry per model: the mods that need work
    let mut targets: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for f in found.findings.iter().filter(|f| f.kind.starts_with("camera_")) {
        targets.entry(&f.model).or_default().extend(f.mods.iter().map(String::as_str));
    }
    let game = if targets.is_empty() { None } else { Some(GameModels::open(game_pak)?) };
    for (model, mods) in targets {
        let skip = |rep: &mut RepairReport, why: &str| rep.skipped.push((model.to_string(), why.to_string()));
        // the mod that wins in the game is the one with the lowest slot
        let Some(winner) = mods.iter().min_by_key(|m| slot(m)) else { continue };
        let Some(addon) = addons.iter().find(|a| a.file_name == *winner) else { continue };
        let Some(g_bytes) = game.as_ref().and_then(|g| g.model(model)) else {
            skip(&mut rep, "the game has no model at this path");
            continue;
        };
        let data = read_vpk_entries(&addon.path)?;
        let Some(m_entry) = data.entries.iter().find(|e| e.path == model) else { continue };
        let m_bytes = match read_entry(&addon.path, &data, m_entry) {
            Ok(b) => b,
            Err(why) => {
                skip(&mut rep, &why);
                continue;
            }
        };
        let result = analyze(&m_bytes, &g_bytes).and_then(|an| if an.needs_repair() { patch_model(&m_bytes, &g_bytes, &an) } else { Err("nothing to repair".into()) });
        match result {
            Ok(bytes) => {
                files.push((model.to_string(), bytes));
                rep.fixed.push(model.to_string());
            }
            Err(why) => skip(&mut rep, &why),
        }
    }
    // keep what an earlier repair put into the package
    for package in [package, legacy] {
        if !package.exists() {
            continue;
        }
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
    fn game_camera_replaces_or_adds() {
        let game = "	AttachmentCameraData = 
	[
		{
			attachment_name = \"near_00\"
		},
	]
";
        let old = "{
	AttachmentCameraData = 
	[
		{
			attachment_name = \"standing_pivot\"
		},
	]
	X = 1
}
";
        let new = with_game_camera(old, game).unwrap();
        assert!(new.contains("near_00") && !new.contains("standing_pivot") && new.contains("X = 1"));
        assert_eq!(camera_names(&camera_block(&new).unwrap()), ["near_00".to_string()].into());
        let none = "{
	X = 1
}
";
        assert!(with_game_camera(none, game).unwrap().contains("near_00"));
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
        let addon = ExistingAddon { file_name: "pak01_dir.vpk".into(), path: p, ours: false, disabled: false, size: 0, installed_at: None, entries: Ok(vec![]) };
        let r = scan(&[addon], None, None);
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
        let (files, rep) = repair_cameras(&game_pak, &addons, &pkg, &PathBuf::from(&out).join(LEGACY_PACKAGE)).unwrap();
        eprintln!("fixed {:?}\nskipped {:?}", rep.fixed, rep.skipped);
        assert!(!files.is_empty());
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        std::fs::write(&pkg, crate::install::vpk::build_vpk(&refs)).unwrap();
    }

    /// Models in zstd (archer) and in the older uncompressed v4 (astro): read, analysed, repaired.
    #[test]
    fn real_zstd_and_v4() {
        let Ok(dir) = std::env::var("DEADLOCK_ADDONS") else { return };
        let addons_dir = PathBuf::from(dir);
        let game_pak = addons_dir.parent().unwrap().join("pak01_dir.vpk");
        let game = GameModels::open(&game_pak).unwrap();
        let mut seen = 0;
        for a in crate::library::scan::scan_addons(&addons_dir).unwrap().iter().filter(|a| !a.disabled) {
            let Ok(data) = read_vpk_entries(&a.path) else { continue };
            for e in data.entries.iter().filter(|e| is_hero_model(&e.path) && (e.path.contains("/archer/") || e.path.contains("/astro/"))) {
                let bytes = read_entry(&a.path, &data, e).unwrap();
                eprintln!("{} {}", a.file_name, e.path);
                for (i, (name, start, size)) in block_table(&bytes).unwrap().into_iter().enumerate() {
                    if &name != b"MDAT" {
                        continue;
                    }
                    let k = Kv3::open(&bytes[start..start + size]);
                    let rg = k.as_ref().map_err(|e| e.clone()).and_then(rig::read_rig);
                    eprintln!("  MDAT #{i}: open {:?} rig {:?}", k.as_ref().err(), rg.as_ref().map(|r| (r.attachments.len(), r.attachments.iter().filter(|a| a.parent == rig::ROOT_BONE).count())));
                }
                let block = data_block(&bytes).unwrap();
                if block.get(..4) == Some(&[0x04, 0x33, 0x56, 0x4B][..]) {
                    // the older layout is rebuilt byte for byte when the text does not change
                    assert_eq!(with_key_values(block, &key_values(block).unwrap()).unwrap(), block);
                }
                let g = game.model(&e.path).expect("game model");
                let an = analyze(&bytes, &g);
                eprintln!("  analysis {an:?}");
                let an = an.unwrap();
                if an.needs_repair() {
                    let fixed = patch_model(&bytes, &g, &an);
                    eprintln!("  repaired: {:?}", fixed.as_ref().map(|b| b.len()));
                    let fixed = fixed.unwrap();
                    assert!(!analyze(&fixed, &g).unwrap().needs_repair());
                }
                seen += 1;
            }
        }
        assert!(seen > 0, "no archer or astro models found");
    }

    /// Against real mods: `DEADLOCK_ADDONS=<addons folder> cargo test … real_addons -- --nocapture`.
    #[test]
    fn real_addons() {
        let Ok(dir) = std::env::var("DEADLOCK_ADDONS") else { return };
        let addons = crate::library::scan::scan_addons(&PathBuf::from(dir)).unwrap();
        let game_pak = PathBuf::from(std::env::var("DEADLOCK_ADDONS").unwrap()).parent().unwrap().join("pak01_dir.vpk");
        let r = scan(&addons, None, Some(&game_pak));
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
