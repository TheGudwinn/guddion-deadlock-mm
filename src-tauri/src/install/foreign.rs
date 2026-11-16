//! Switch a mod installed by another manager on or off the way that manager does: the game loads only
//! `pak##_dir.vpk`, so a mod is turned off by renaming its files to `<catalog id>_<name>` and back on by
//! giving them free `pak##_dir.vpk` names again. A file with no known catalog id gets the same `.disabled`
//! suffix this manager uses for its own files. Every rename goes through the journal, so the whole
//! switch can be undone with one click and a failure half way restores everything.
use super::error::InstallError;
use super::installer::Ctx;
use super::journal::{ensure_within, Op};
use super::pak_order::{is_ours, DISABLED_SUFFIX};
use crate::library::foreign_id::is_loaded_name;
use std::collections::HashSet;

fn now() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn pak_name(n: u32) -> String {
    format!("pak{n:02}_dir.vpk")
}

fn next_free(taken: &HashSet<String>) -> String {
    (1u32..).map(pak_name).find(|n| !taken.contains(n)).expect("an unused pak number exists")
}

/// Which renames switch `files` on or off. `existing` are all file names in the addons folder.
/// Files already in the requested state are left out. Pure, so the naming rules are easy to test.
pub fn plan_toggle(existing: &HashSet<String>, id: Option<&str>, files: &[String], enabled: bool) -> Result<Vec<(String, String)>, InstallError> {
    let mut taken = existing.clone();
    let mut moves = Vec::new();
    for f in files {
        if enabled {
            if is_loaded_name(f) {
                continue;
            }
            let base = f.strip_suffix(DISABLED_SUFFIX).unwrap_or(f);
            let base = id.and_then(|i| base.strip_prefix(&format!("{i}_"))).unwrap_or(base);
            // Keep the original load slot when it is free, otherwise the lowest free one.
            let target = if is_loaded_name(base) && !taken.contains(base) { base.to_string() } else { next_free(&taken) };
            taken.insert(target.clone());
            moves.push((f.clone(), target));
        } else {
            if !is_loaded_name(f) {
                continue;
            }
            let to = match id {
                Some(i) => format!("{i}_{f}"),
                None => format!("{f}{DISABLED_SUFFIX}"),
            };
            if taken.contains(&to) {
                return Err(InstallError::AlreadyExists(to.into()));
            }
            taken.insert(to.clone());
            moves.push((f.clone(), to));
        }
    }
    Ok(moves)
}

/// A mod split over chunk files (`name_000.vpk`) cannot be renamed safely: the directory file refers to
/// the chunks by name.
fn has_chunks(existing: &HashSet<String>, file: &str) -> bool {
    let base = file.strip_suffix(DISABLED_SUFFIX).unwrap_or(file);
    let Some(stem) = base.strip_suffix("_dir.vpk") else { return false };
    let prefix = format!("{stem}_");
    existing.iter().any(|n| n.starts_with(&prefix) && n.ends_with(".vpk") && !n.ends_with("_dir.vpk"))
}

/// Returns the journal batch id (empty when nothing had to change).
pub fn toggle_foreign(ctx: &Ctx, id: Option<&str>, files: &[String], enabled: bool) -> Result<String, InstallError> {
    let dir = &ctx.game.addons_dir;
    let existing: HashSet<String> = std::fs::read_dir(dir)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();

    for f in files {
        let plain = !f.is_empty() && !f.contains(['/', '\\']) && f != "." && f != "..";
        if !plain || is_ours(f) {
            return Err(InstallError::OutsideRoot(dir.join(f)));
        }
        ensure_within(dir, &dir.join(f))?;
        if !dir.join(f).is_file() {
            return Err(InstallError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{f} is missing"))));
        }
        if has_chunks(&existing, f) {
            return Err(InstallError::ExternalChunks(f.clone()));
        }
    }

    let moves = plan_toggle(&existing, id, files, enabled)?;
    if moves.is_empty() {
        return Ok(String::new());
    }
    let tag: String = id.unwrap_or("x").chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    // The "g" keeps the id from being read as one of this manager's own mod ids when naming the action.
    let batch = format!("foreign-g{tag}-{}", now());
    let result = (|| {
        for (from, to) in &moves {
            let (from, to) = (dir.join(from), dir.join(to));
            ensure_within(dir, &to)?;
            ctx.journal.record(&batch, Op::Rename { from: from.clone(), to: to.clone() })?;
            std::fs::rename(&from, &to)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = ctx.journal.rollback_batch(&batch);
    }
    result.map(|_: ()| batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn files(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn disabling_prefixes_the_catalog_id_and_skips_files_already_off() {
        let ex = set(&["pak01_dir.vpk", "569650_pak02_dir.vpk"]);
        let m = plan_toggle(&ex, Some("569650"), &files(&["pak01_dir.vpk", "569650_pak02_dir.vpk"]), false).unwrap();
        assert_eq!(m, vec![("pak01_dir.vpk".to_string(), "569650_pak01_dir.vpk".to_string())]);
    }

    #[test]
    fn disabling_without_an_id_uses_the_disabled_suffix() {
        let m = plan_toggle(&set(&["pak05_dir.vpk"]), None, &files(&["pak05_dir.vpk"]), false).unwrap();
        assert_eq!(m, vec![("pak05_dir.vpk".to_string(), "pak05_dir.vpk.disabled".to_string())]);
    }

    #[test]
    fn enabling_keeps_a_free_original_slot_else_takes_the_lowest_free_one() {
        let ex = set(&["pak01_dir.vpk", "pak03_dir.vpk", "7_pak01_dir.vpk", "7_pak06_dir.vpk"]);
        let m = plan_toggle(&ex, Some("7"), &files(&["7_pak01_dir.vpk", "7_pak06_dir.vpk"]), true).unwrap();
        // pak01 is taken -> lowest free is pak02; pak06 is free -> kept.
        assert_eq!(m, vec![("7_pak01_dir.vpk".to_string(), "pak02_dir.vpk".to_string()), ("7_pak06_dir.vpk".to_string(), "pak06_dir.vpk".to_string())]);
    }

    #[test]
    fn enabling_two_files_of_one_mod_never_picks_the_same_slot() {
        let m = plan_toggle(&set(&["9_a_dir.vpk", "9_b_dir.vpk"]), Some("9"), &files(&["9_a_dir.vpk", "9_b_dir.vpk"]), true).unwrap();
        assert_eq!(m[0].1, "pak01_dir.vpk");
        assert_eq!(m[1].1, "pak02_dir.vpk");
    }

    #[test]
    fn enabling_a_suffixed_file_restores_its_name_and_refuses_a_clash_when_disabling() {
        let m = plan_toggle(&set(&["pak05_dir.vpk.disabled"]), None, &files(&["pak05_dir.vpk.disabled"]), true).unwrap();
        assert_eq!(m, vec![("pak05_dir.vpk.disabled".to_string(), "pak05_dir.vpk".to_string())]);
        let err = plan_toggle(&set(&["pak05_dir.vpk", "5_pak05_dir.vpk"]), Some("5"), &files(&["pak05_dir.vpk"]), false).unwrap_err();
        assert!(matches!(err, InstallError::AlreadyExists(_)));
    }

    #[test]
    fn chunked_mods_are_detected() {
        let ex = set(&["pak01_dir.vpk", "pak01_000.vpk", "pak02_dir.vpk"]);
        assert!(has_chunks(&ex, "pak01_dir.vpk"));
        assert!(!has_chunks(&ex, "pak02_dir.vpk"));
    }
}
