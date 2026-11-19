//! Checks for the usual reasons a modded game misbehaves (mods ignored, broken camera, leftovers) and
//! the fixes for them. Every change to a game file is journaled with a backup, so it can be undone.
use super::prefs::{apply_block, GamePrefs, BLOCK_END, BLOCK_START};
use crate::install::error::InstallError;
use crate::install::foreign::toggle_foreign;
use crate::install::gameinfo::{ensure_addons_path, ensure_pinned, write_atomic, PatchResult};
use crate::install::installer::{set_enabled, Ctx};
use crate::install::journal::{ensure_within, Op};
use crate::install::pak_order::is_ours;
use crate::library::scan::scan_addons;
use crate::steam::locate::GameInstall;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    /// Stable name the interface translates.
    pub kind: String,
    /// `error`, `warn` or `info`.
    pub severity: &'static str,
    /// Addon files or lines the finding is about.
    pub items: Vec<String>,
    /// Fix the interface can offer: `patch_gameinfo`, `disable`, `clean_temp`, `comment_autoexec`.
    pub fix: Option<&'static str>,
}

fn finding(kind: &str, severity: &'static str, items: Vec<String>, fix: Option<&'static str>) -> Finding {
    Finding { kind: kind.into(), severity, items, fix }
}

pub fn autoexec_path(game: &GameInstall) -> PathBuf {
    game.citadel_dir.join("cfg").join("autoexec.cfg")
}

/// A console variable that changes the camera or field of view.
fn is_camera_cvar(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("//") {
        return false;
    }
    let word = t.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let word = word.trim_start_matches('+');
    word.starts_with("cam_") || word.starts_with("fov") || word.starts_with("cl_fov") || word.starts_with("camera")
}

/// A path inside a mod that replaces camera behaviour.
fn is_camera_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("camera") || p.contains("/cam_") || p.ends_with(".vcam")
}

fn temp_files(addons: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(addons) else { return Vec::new() };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part") || n.ends_with(".tmp") || n.contains(".tmp."))
        .collect();
    v.sort();
    v
}

pub fn run_checks(game: &GameInstall) -> Vec<Finding> {
    let mut out = Vec::new();

    match std::fs::read(&game.gameinfo).ok().and_then(|b| String::from_utf8(b).ok()) {
        None => out.push(finding("gameinfo_unreadable", "error", vec![], None)),
        Some(text) => match ensure_addons_path(&text) {
            PatchResult::AlreadyPresent => {
                // the addons path is there; a folder listed before `Game citadel` must not become the game's own folder
                if matches!(ensure_pinned(&text), PatchResult::Patched(_)) {
                    out.push(finding("gameinfo_unpinned", "error", vec![], Some("patch_gameinfo")));
                }
            }
            PatchResult::Patched(_) => out.push(finding("gameinfo_unpatched", "error", vec![], Some("patch_gameinfo"))),
            PatchResult::Unrecognized => out.push(finding("gameinfo_unrecognized", "error", vec![], None)),
        },
    }

    let addons = scan_addons(&game.addons_dir).unwrap_or_default();
    let broken: Vec<String> = addons.iter().filter(|a| a.entries.is_err()).map(|a| a.file_name.clone()).collect();
    if !broken.is_empty() {
        out.push(finding("broken_vpk", "warn", broken, Some("disable")));
    }

    let camera: Vec<String> = addons
        .iter()
        .filter(|a| !a.disabled)
        .filter(|a| a.entries.as_ref().map(|e| e.iter().any(|f| is_camera_path(&f.path))).unwrap_or(false))
        .map(|a| a.file_name.clone())
        .collect();
    if !camera.is_empty() {
        out.push(finding("camera_mod", "warn", camera, Some("disable")));
    }

    if let Ok(text) = std::fs::read_to_string(autoexec_path(game)) {
        let lines: Vec<String> = managed_free(&text).lines().filter(|l| is_camera_cvar(l)).map(|l| l.trim().to_string()).collect();
        if !lines.is_empty() {
            out.push(finding("autoexec_camera", "warn", lines, Some("comment_autoexec")));
        }
    }

    let temp = temp_files(&game.addons_dir);
    if !temp.is_empty() {
        out.push(finding("temp_files", "info", temp, Some("clean_temp")));
    }

    let disabled_ours = addons.iter().filter(|a| a.ours && a.disabled).count();
    if disabled_ours > 0 && addons.iter().all(|a| a.disabled) {
        out.push(finding("all_off", "info", vec![], None));
    }
    out
}

/// The text without our own managed block (its lines are not the user's).
fn managed_free(text: &str) -> String {
    apply_block(text, None)
}

pub(crate) fn stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// Replace `path` with `new`, keeping a verified backup and a journal entry for undo.
pub(crate) fn replace_journaled(ctx: &Ctx, batch: &str, path: &Path, old: &[u8], new: &[u8]) -> Result<(), InstallError> {
    std::fs::create_dir_all(ctx.backups)?;
    let backup = ctx.backups.join(format!("{batch}_{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    write_atomic(&backup, old)?;
    if std::fs::read(&backup)? != old {
        return Err(InstallError::Journal("backup does not match the original".into()));
    }
    ctx.journal.record(batch, Op::ReplaceFile { path: path.to_path_buf(), backup })?;
    write_atomic(path, new)?;
    Ok(())
}

pub fn fix_gameinfo(ctx: &Ctx) -> Result<String, InstallError> {
    let batch = format!("fix-gameinfo-{}", stamp());
    let bytes = std::fs::read(&ctx.game.gameinfo)?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| InstallError::GameinfoUnrecognized)?;
    match ensure_addons_path(&text) {
        PatchResult::AlreadyPresent => match ensure_pinned(&text) {
            PatchResult::Patched(new) => {
                replace_journaled(ctx, &batch, &ctx.game.gameinfo, &bytes, new.as_bytes())?;
                Ok(batch)
            }
            _ => Ok(String::new()),
        },
        PatchResult::Unrecognized => Err(InstallError::GameinfoUnrecognized),
        PatchResult::Patched(new) => {
            if ensure_addons_path(&new) != PatchResult::AlreadyPresent {
                return Err(InstallError::GameinfoUnrecognized);
            }
            replace_journaled(ctx, &batch, &ctx.game.gameinfo, &bytes, new.as_bytes())?;
            Ok(batch)
        }
    }
}

/// Comment out camera / field-of-view lines in `autoexec.cfg`.
pub fn fix_autoexec_camera(ctx: &Ctx) -> Result<String, InstallError> {
    let path = autoexec_path(ctx.game);
    let batch = format!("fix-autoexec-{}", stamp());
    let bytes = std::fs::read(&path)?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| InstallError::GameinfoUnrecognized)?;
    let mut in_block = false;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with(BLOCK_START) {
            in_block = true;
        }
        if !in_block && is_camera_cvar(line) {
            out.push_str("// (disabled by Guddion) ");
        }
        if t.starts_with(BLOCK_END) {
            in_block = false;
        }
        out.push_str(line);
    }
    if out == text {
        return Ok(String::new());
    }
    replace_journaled(ctx, &batch, &path, &bytes, out.as_bytes())?;
    Ok(batch)
}

/// Write (or remove) the performance block of `autoexec.cfg` for `prefs`.
pub fn write_performance(ctx: &Ctx, prefs: &GamePrefs) -> Result<Option<String>, InstallError> {
    let path = autoexec_path(ctx.game);
    let batch = format!("performance-{}", stamp());
    let block = prefs.autoexec_block();
    match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes.clone()).map_err(|_| InstallError::GameinfoUnrecognized)?;
            let new = apply_block(&text, block.as_deref());
            if new == text {
                return Ok(None);
            }
            replace_journaled(ctx, &batch, &path, &bytes, new.as_bytes())?;
            Ok(Some(batch))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let Some(b) = block else { return Ok(None) };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            ctx.journal.record(&batch, Op::CreateFile { path: path.clone() })?;
            write_atomic(&path, b.as_bytes())?;
            Ok(Some(batch))
        }
        Err(e) => Err(e.into()),
    }
}

/// Switch addon files off: ours get the `.disabled` suffix, other managers' files follow their convention.
pub fn disable_addons(ctx: &Ctx, files: &[String]) -> Result<usize, InstallError> {
    let mut n = 0;
    let mut foreign: Vec<String> = Vec::new();
    for f in files {
        ensure_within(&ctx.game.addons_dir, &ctx.game.addons_dir.join(f))?;
        if is_ours(f) {
            if !set_enabled(ctx, f, false)?.is_empty() {
                n += 1;
            }
        } else {
            foreign.push(f.clone());
        }
    }
    if !foreign.is_empty() {
        toggle_foreign(ctx, None, &foreign, false)?;
        n += foreign.len();
    }
    Ok(n)
}

/// Delete download leftovers from the addons folder. Only the app's own temporary names are touched.
pub fn clean_temp(game: &GameInstall) -> std::io::Result<usize> {
    let mut n = 0;
    for name in temp_files(&game.addons_dir) {
        std::fs::remove_file(game.addons_dir.join(name))?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_cvars() {
        assert!(is_camera_cvar("cam_idealdist 200"));
        assert!(is_camera_cvar("  fov_desired 110"));
        assert!(is_camera_cvar("+cl_fov 90"));
        assert!(!is_camera_cvar("// fov 90"));
        assert!(!is_camera_cvar("fps_max 144"));
        assert!(!is_camera_cvar("bind f fov_x"));
    }

    #[test]
    fn camera_paths() {
        assert!(is_camera_path("scripts/camera/default.vdata"));
        assert!(is_camera_path("panorama/images/x.vcam"));
        assert!(!is_camera_path("models/heroes/abrams/abrams.vmdl_c"));
    }

    fn fake_game(dir: &Path) -> GameInstall {
        let citadel = dir.join("game").join("citadel");
        std::fs::create_dir_all(citadel.join("cfg")).unwrap();
        std::fs::create_dir_all(citadel.join("addons")).unwrap();
        std::fs::write(
            citadel.join("gameinfo.gi"),
            "\"GameInfo\"
{
	FileSystem
	{
		SearchPaths
		{
			Game	citadel
		}
	}
}
",
        )
        .unwrap();
        GameInstall { root: dir.to_path_buf(), citadel_dir: citadel.clone(), addons_dir: citadel.join("addons"), gameinfo: citadel.join("gameinfo.gi"), build_id: None }
    }

    #[test]
    fn finds_and_fixes_the_usual_problems() {
        use crate::install::extract::Limits;
        use crate::install::journal::Journal;
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_game(tmp.path());
        std::fs::write(autoexec_path(&game), "bind x jump
cam_idealdist 300
").unwrap();
        std::fs::write(game.addons_dir.join("x.vpk.part"), b"half").unwrap();

        let kinds = |g: &GameInstall| run_checks(g).into_iter().map(|f| f.kind).collect::<Vec<_>>();
        let k = kinds(&game);
        assert!(k.contains(&"gameinfo_unpatched".to_string()) && k.contains(&"autoexec_camera".to_string()) && k.contains(&"temp_files".to_string()), "{k:?}");

        let journal = Journal::open(tmp.path().join("journal.jsonl"));
        let (staging, backups) = (tmp.path().join("staging"), tmp.path().join("backups"));
        let ctx = Ctx { game: &game, staging: &staging, backups: &backups, journal: &journal, limits: Limits::default() };
        fix_gameinfo(&ctx).unwrap();
        fix_autoexec_camera(&ctx).unwrap();
        assert_eq!(clean_temp(&game).unwrap(), 1);
        assert!(kinds(&game).is_empty(), "{:?}", kinds(&game));
        let ae = std::fs::read_to_string(autoexec_path(&game)).unwrap();
        assert!(ae.contains("bind x jump
") && ae.contains("// (disabled by Guddion) cam_idealdist 300"), "{ae:?}");

        // The journal can undo what the fixes changed.
        let batches: Vec<String> = journal.entries().unwrap().into_iter().map(|e| e.batch).collect();
        for b in batches.iter().rev() {
            journal.rollback_batch(b).unwrap();
        }
        assert!(kinds(&game).contains(&"gameinfo_unpatched".to_string()));
    }

    #[test]
    fn performance_block_is_written_and_removed() {
        use crate::game::prefs::VSync;
        use crate::install::extract::Limits;
        use crate::install::journal::Journal;
        let tmp = tempfile::tempdir().unwrap();
        let game = fake_game(tmp.path());
        let journal = Journal::open(tmp.path().join("journal.jsonl"));
        let (staging, backups) = (tmp.path().join("staging"), tmp.path().join("backups"));
        let ctx = Ctx { game: &game, staging: &staging, backups: &backups, journal: &journal, limits: Limits::default() };
        let p = GamePrefs { fps_max: 120, vsync: VSync::Off, ..Default::default() };
        assert!(write_performance(&ctx, &p).unwrap().is_some());
        let text = std::fs::read_to_string(autoexec_path(&game)).unwrap();
        assert!(text.contains("fps_max 120") && text.contains("mat_vsync 0"));
        assert!(write_performance(&ctx, &p).unwrap().is_none(), "same settings change nothing");
        write_performance(&ctx, &GamePrefs::default()).unwrap();
        assert!(!std::fs::read_to_string(autoexec_path(&game)).unwrap().contains("fps_max"));
    }
}
