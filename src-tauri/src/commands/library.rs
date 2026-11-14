//! Install / library commands.
use super::{err, AppState};
use crate::install::extract::Limits;
use crate::install::installer::{install_mod as run_install, reorder_files, toggle_mod, uninstall_files, Ctx, ReorderItem};
use crate::install::journal::Journal;
use crate::install::pak_order::{first_free_slot, is_ours, mod_id_of, slot_of, DISABLED_SUFFIX};
use crate::library::scan::{real_conflicts, scan_addons, Overlap};
use crate::library::store;
use crate::sources::ModFile;
use crate::steam::locate::GameInstall;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

pub(super) fn game(state: &AppState) -> Result<GameInstall, String> {
    state.game()?.ok_or_else(|| "game_not_found".to_string())
}

pub(super) struct Dirs {
    pub journal: PathBuf,
    pub staging: PathBuf,
    pub backups: PathBuf,
    pub downloads: PathBuf,
}

pub(super) fn dirs(state: &AppState) -> Dirs {
    Dirs {
        journal: state.app_dir.join("journal.jsonl"),
        staging: state.app_dir.join("staging"),
        backups: state.app_dir.join("backups"),
        downloads: state.app_dir.join("downloads"),
    }
}

#[derive(Serialize)]
pub struct InstallResult {
    pub mod_id: i64,
    pub name: String,
    pub slot: u8,
    pub overlaps: Vec<Overlap>,
}

#[derive(Serialize)]
pub struct InstalledFile {
    pub name: String,
    pub size: u64,
    pub present: bool,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct InstalledMod {
    pub id: i64,
    pub source: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub remote_id: Option<String>,
    pub slot: Option<u8>,
    pub enabled: bool,
    /// Game build the mod was installed on (for the patch warning).
    pub game_build: Option<String>,
    pub files: Vec<InstalledFile>,
}

#[derive(Serialize)]
pub struct ForeignAddon {
    pub name: String,
    pub size: u64,
    pub entries: Option<usize>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct LastAction {
    pub kind: String,
    pub mod_name: Option<String>,
}

pub(super) use crate::queue::safe_file_name;

fn batch_parts(batch: &str) -> (&str, Option<i64>) {
    let mut it = batch.split('-');
    let kind = it.next().unwrap_or("");
    (kind, it.next().and_then(|s| s.parse().ok()))
}

/// Installs an already downloaded archive: records the mod, picks the first free slot, copies the files.
/// Called by the download queue once a transfer has finished.
pub(super) async fn perform_install(state: &AppState, remote_id: &str, mod_name: &str, version: Option<String>, file: &ModFile, dest: &Path) -> Result<InstallResult, String> {
    let _guard = state.busy.lock().await;
    let game = game(state)?;
    let d = dirs(state);
    let source = state.source();
    let db = state.db().await?;

    if let Some(existing) = store::find_by_remote(&db.pool, source.id(), remote_id).await.map_err(err)? {
        if existing.state == "installed" {
            return Err("already_installed".into());
        }
        store::delete_mod(&db.pool, existing.id).await.map_err(err)?; // previously removed: start clean
    }

    let db_id = store::insert_mod(&db.pool, source.id(), remote_id, mod_name, version.as_deref()).await.map_err(err)?;
    let used: Vec<u8> = scan_addons(&game.addons_dir).map_err(err)?.iter().filter_map(|a| slot_of(&a.file_name)).collect();
    let slot = match first_free_slot(&used) {
        Some(s) => s,
        None => {
            let _ = store::delete_mod(&db.pool, db_id).await;
            return Err("no_free_slots".into());
        }
    };

    let g = game.clone();
    let archive = dest.to_path_buf();
    let outcome = tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&d.journal);
        let ctx = Ctx { game: &g, staging: &d.staging, backups: &d.backups, journal: &journal, limits: Limits::default() };
        run_install(&ctx, &archive, db_id, slot)
    })
    .await
    .map_err(err)?;

    match outcome {
        Ok(out) => {
            for f in &out.files {
                store::add_file(&db.pool, db_id, &f.addon_name, &f.sha256, f.size as i64).await.map_err(err)?;
            }
            store::set_batch(&db.pool, db_id, &out.batch).await.map_err(err)?;
            let meta = store::Meta { version, file_id: Some(file.id.clone()), file_md5: file.md5.clone(), game_build: game.build_id.clone() };
            store::set_meta(&db.pool, db_id, &meta).await.map_err(err)?;
            Ok(InstallResult { mod_id: db_id, name: mod_name.to_string(), slot, overlaps: out.overlaps })
        }
        Err(e) => {
            let _ = store::delete_mod(&db.pool, db_id).await;
            Err(match e {
                crate::install::error::InstallError::AlreadyInstalled(n) => format!("duplicate:{n}"),
                other => other.to_string(),
            })
        }
    }
}

#[tauri::command]
pub async fn list_installed(state: State<'_, AppState>) -> Result<Vec<InstalledMod>, String> {
    installed_mods(&state).await
}

/// Installed mods with their on-disk state; also drops records whose files are gone.
pub(super) async fn installed_mods(state: &AppState) -> Result<Vec<InstalledMod>, String> {
    let db = state.db().await?;
    let addons = state.game().ok().flatten().map(|g| g.addons_dir);
    let mut out = Vec::new();
    for m in store::list_mods(&db.pool).await.map_err(err)? {
        if m.state != "installed" {
            continue;
        }
        let names = store::files_of(&db.pool, m.id).await.map_err(err)?;
        let mut files = Vec::new();
        for n in names {
            let (present, enabled, size) = match &addons {
                Some(dir) => {
                    let on = dir.join(&n);
                    let off = dir.join(format!("{n}.disabled"));
                    match (std::fs::metadata(&on), std::fs::metadata(&off)) {
                        (Ok(md), _) => (true, true, md.len()),
                        (_, Ok(md)) => (true, false, md.len()),
                        _ => (false, false, 0),
                    }
                }
                None => (false, false, 0),
            };
            files.push(InstalledFile { name: n, size, present, enabled });
        }
        // Rows whose files are gone (e.g. after an undo) are stale; drop them.
        if addons.is_some() && !files.is_empty() && files.iter().all(|f| !f.present) {
            store::delete_mod(&db.pool, m.id).await.map_err(err)?;
            continue;
        }
        let game_build = store::install_meta(&db.pool, m.id).await.map_err(err)?.game_build;
        out.push(InstalledMod {
            game_build,
            id: m.id,
            source: m.source,
            slot: files.first().and_then(|f| slot_of(&f.name)),
            enabled: files.iter().any(|f| f.enabled),
            name: m.name,
            version: m.version,
            remote_id: m.remote_id,
            files,
        });
    }
    out.sort_by_key(|m| m.slot.unwrap_or(u8::MAX));
    Ok(out)
}

/// Addons in the game folder that this app did not install (other managers, manual copies).
#[tauri::command]
pub async fn list_foreign_addons(state: State<'_, AppState>) -> Result<Vec<ForeignAddon>, String> {
    let Some(g) = state.game().ok().flatten() else { return Ok(vec![]) };
    tokio::task::spawn_blocking(move || {
        scan_addons(&g.addons_dir).map_err(err).map(|v| {
            v.into_iter()
                .filter(|a| !a.ours)
                .map(|a| ForeignAddon {
                    name: a.file_name,
                    size: a.size,
                    entries: a.entries.as_ref().ok().map(|e| e.len()),
                    error: a.entries.err(),
                })
                .collect()
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub async fn set_mod_enabled(state: State<'_, AppState>, id: i64, enabled: bool) -> Result<(), String> {
    let _guard = state.busy.lock().await;
    let game = game(&state)?;
    let d = dirs(&state);
    let names = store::files_of(&state.db().await?.pool, id).await.map_err(err)?;
    tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&d.journal);
        let ctx = Ctx { game: &game, staging: &d.staging, backups: &d.backups, journal: &journal, limits: Limits::default() };
        toggle_mod(&ctx, id, &names, enabled).map(|_| ())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn uninstall_mod(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let _guard = state.busy.lock().await;
    let game = game(&state)?;
    let d = dirs(&state);
    let db = state.db().await?;
    let names = store::files_of(&db.pool, id).await.map_err(err)?;
    tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&d.journal);
        let ctx = Ctx { game: &game, staging: &d.staging, backups: &d.backups, journal: &journal, limits: Limits::default() };
        uninstall_files(&ctx, id, &names).map(|_| ())
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    store::set_state(&db.pool, id, "removed").await.map_err(err)
}

fn last_batch(path: &Path) -> Result<Option<String>, String> {
    let entries = Journal::open(path).entries().map_err(err)?;
    Ok(entries.into_iter().rev().find(|e| !e.rolled_back).map(|e| e.batch))
}

/// What "Undo" would revert right now.
#[tauri::command]
pub async fn last_action(state: State<'_, AppState>) -> Result<Option<LastAction>, String> {
    let Some(batch) = last_batch(&dirs(&state).journal)? else { return Ok(None) };
    let (kind, id) = batch_parts(&batch);
    let mod_name = match (kind, id) {
        ("profile", Some(id)) => crate::profiles::store::name_of(&state.db().await?.pool, id).await.map_err(err)?,
        (_, Some(id)) => store::get_mod(&state.db().await?.pool, id).await.map_err(err)?.map(|m| m.name),
        _ => None,
    };
    Ok(Some(LastAction { kind: kind.to_string(), mod_name }))
}

/// Revert the most recent file change. Only the newest action can be undone, so the game folder
/// always returns through the same states it went through.
#[tauri::command]
pub async fn undo_last(state: State<'_, AppState>) -> Result<(), String> {
    let _guard = state.busy.lock().await;
    let d = dirs(&state);
    let Some(batch) = last_batch(&d.journal)? else { return Ok(()) };
    let journal_path = d.journal.clone();
    let b = batch.clone();
    tokio::task::spawn_blocking(move || Journal::open(journal_path).rollback_batch(&b)).await.map_err(err)?.map_err(err)?;

    let db = state.db().await?;
    match batch_parts(&batch) {
        ("install", Some(id)) => store::delete_mod(&db.pool, id).await.map_err(err)?,
        ("uninstall", Some(id)) => store::set_state(&db.pool, id, "installed").await.map_err(err)?,
        // After undoing a profile switch the library no longer matches any profile.
        ("profile", _) => crate::profiles::store::set_active(&db.pool, None).await.map_err(err)?,
        ("reorder", _) => resync_names(&state).await?,
        ("update", Some(id)) => super::mod_updates::restore_after_undo(&state, id, &batch).await?,
        _ => {}
    }
    Ok(())
}

/// Makes the stored file names match the files on disk after slots were renumbered or restored.
/// Disk is the truth; a mod is only touched when its file count matches (a hand-deleted file is left alone).
pub(super) async fn resync_names(state: &AppState) -> Result<(), String> {
    let Some(game) = state.game().ok().flatten() else { return Ok(()) };
    let db = state.db().await?;
    let mut by_mod: std::collections::HashMap<i64, Vec<String>> = Default::default();
    for e in std::fs::read_dir(&game.addons_dir).map_err(err)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_ours(&name) {
            continue;
        }
        let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(&name).to_string();
        if let Some(id) = mod_id_of(&base) {
            by_mod.entry(id).or_default().push(base);
        }
    }
    for (id, mut names) in by_mod {
        names.sort_by_key(|n| slot_of(n).unwrap_or(u8::MAX));
        let rows = store::file_rows(&db.pool, id).await.map_err(err)?;
        if rows.len() != names.len() {
            continue;
        }
        for ((row, old), new) in rows.iter().zip(&names) {
            if old != new {
                store::set_file_name(&db.pool, *row, new).await.map_err(err)?;
            }
        }
    }
    Ok(())
}

/// Sets the load priority of all installed mods. `order` lists every installed mod id, highest
/// priority first. Files are renamed so slots follow the order; one undoable action.
#[tauri::command]
pub async fn reorder_mods(state: State<'_, AppState>, order: Vec<i64>) -> Result<(), String> {
    let _guard = state.busy.lock().await;
    let game = game(&state)?;
    let d = dirs(&state);
    let installed = installed_mods(&state).await?;

    let mut want = order.clone();
    want.sort_unstable();
    let mut have: Vec<i64> = installed.iter().map(|m| m.id).collect();
    have.sort_unstable();
    if want != have {
        return Err("order_mismatch".into());
    }
    let items: Vec<ReorderItem> = order
        .iter()
        .filter_map(|id| installed.iter().find(|m| m.id == *id))
        .map(|m| ReorderItem { mod_id: m.id, files: m.files.iter().map(|f| f.name.clone()).collect() })
        .collect();

    let journal_path = d.journal.clone();
    tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&journal_path);
        let ctx = Ctx { game: &game, staging: &d.staging, backups: &d.backups, journal: &journal, limits: Limits::default() };
        reorder_files(&ctx, &items).map(|_| ())
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    resync_names(&state).await
}

#[derive(Serialize)]
pub struct LibraryStats {
    /// Files in the game's addons folder that this app did not install.
    pub foreign: usize,
    /// Size of the addons this app installed.
    pub disk_bytes: u64,
    /// Paths provided by several enabled addons with different content (one of them silently wins).
    pub conflicts: usize,
}

#[tauri::command]
pub async fn library_stats(state: State<'_, AppState>) -> Result<LibraryStats, String> {
    let Some(g) = state.game().ok().flatten() else {
        return Ok(LibraryStats { foreign: 0, disk_bytes: 0, conflicts: 0 });
    };
    tokio::task::spawn_blocking(move || {
        let all = scan_addons(&g.addons_dir).map_err(err)?;
        Ok(LibraryStats {
            foreign: all.iter().filter(|a| !a.ours).count(),
            disk_bytes: all.iter().filter(|a| a.ours).map(|a| a.size).sum(),
            conflicts: real_conflicts(&all),
        })
    })
    .await
    .map_err(err)?
}
