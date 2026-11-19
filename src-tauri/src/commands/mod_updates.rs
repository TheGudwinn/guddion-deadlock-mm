//! Updating installed mods: check for newer files, replace files atomically, undo.
use super::library::{dirs, game, installed_mods, safe_file_name};
use super::{err, AppState};
use crate::install::extract::Limits;
use crate::install::installer::{toggle_mod, update_files, Ctx};
use crate::install::journal::Journal;
use crate::install::pak_order::{slot_of, DISABLED_SUFFIX};
use crate::library::scan::Overlap;
use crate::library::store::{self, Meta};
use crate::library::updates::{find_update, installable, InstalledFile};
use crate::net::download::Progress;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};

#[derive(Serialize)]
pub struct ModUpdate {
    pub mod_id: i64,
    pub name: String,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub file_name: String,
}

#[derive(Serialize)]
pub struct UpdateReport {
    pub updates: Vec<ModUpdate>,
    /// Mods that could be compared (installed by a version of the app that records the source file).
    pub checked: usize,
    /// Mods whose page could not be loaded.
    pub failed: usize,
    /// Some answers came from the offline cache, so "no update" may be out of date.
    pub offline: bool,
}

#[derive(Serialize)]
pub struct UpdateResult {
    pub name: String,
    pub slot: u8,
    pub overlaps: Vec<Overlap>,
}

#[derive(Serialize, Clone)]
struct ProgressEvent {
    stage: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

/// Compares every installed GameBanana mod with its page. Pages are fetched one by one
/// (the source throttles itself), a failing page does not stop the others.
#[tauri::command]
pub async fn check_mod_updates(state: State<'_, AppState>) -> Result<UpdateReport, String> {
    let db = state.db().await?;
    let source = state.source();
    let mut report = UpdateReport { updates: vec![], checked: 0, failed: 0, offline: false };

    for m in installed_mods(&state).await? {
        let (Some(src), Some(remote)) = (m.source.as_deref(), m.remote_id.as_deref()) else { continue };
        if src != source.id() {
            continue;
        }
        let meta = store::install_meta(&db.pool, m.id).await.map_err(err)?;
        let Some(file_id) = meta.file_id.as_deref() else { continue }; // installed before this was recorded
        report.checked += 1;
        match source.get_mod_details(remote).await {
            Ok(d) => {
                report.offline |= d.cached_at.is_some();
                let installed = InstalledFile { file_id, version: meta.version.as_deref(), md5: meta.file_md5.as_deref() };
                if let Some(f) = find_update(&installed, d.summary.version.as_deref(), &d.files, installable) {
                    report.updates.push(ModUpdate {
                        mod_id: m.id,
                        name: m.name.clone(),
                        installed_version: meta.version.clone(),
                        latest_version: d.summary.version.clone(),
                        file_name: f.name.clone(),
                    });
                }
            }
            Err(_) => report.failed += 1,
        }
    }
    Ok(report)
}

/// Downloads the newer file and swaps it in as one undoable action. The mod keeps its slot and
/// its enabled/disabled state.
#[tauri::command]
pub async fn update_mod(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<UpdateResult, String> {
    let name = installed_mods(&state).await.ok().and_then(|v| v.into_iter().find(|m| m.id == id)).map(|m| m.name).unwrap_or_default();
    let r = update_mod_impl(app, &state, id).await;
    log_update(&state, &name, None, &r);
    r
}

/// Puts the end of an update into the history (a request for a choice of file is not an end).
fn log_update(state: &AppState, name: &str, file: Option<&str>, r: &Result<UpdateResult, String>) {
    let (st, detail, shown) = match r {
        Ok(u) => ("done", None, u.name.clone()),
        Err(e) if e == "choose_file" => return,
        Err(e) => ("failed", Some(e.clone()), name.to_string()),
    };
    crate::history::push(&state.app_dir, crate::history::Entry { at: crate::history::now(), kind: "update".into(), name: shown, file: file.map(str::to_string), state: st.into(), detail });
}

async fn update_mod_impl(app: AppHandle, state: &AppState, id: i64) -> Result<UpdateResult, String> {
    let _guard = state.busy.lock().await;
    let game = game(&state)?;
    let d = dirs(&state);
    let db = state.db().await?;
    let source = state.source();

    let installed = installed_mods(&state).await?;
    let m = installed.iter().find(|m| m.id == id).ok_or("mod_not_found")?;
    let remote = m.remote_id.clone().ok_or("update_no_source")?;
    let slot = m.slot.ok_or("update_no_source")?;
    let old_names: Vec<String> = m.files.iter().map(|f| f.name.clone()).collect();
    let old_meta = store::install_meta(&db.pool, id).await.map_err(err)?;
    let file_id = old_meta.file_id.clone().ok_or("update_no_source")?;

    let details = source.get_mod_details(&remote).await.map_err(err)?;
    let installed_file = InstalledFile { file_id: &file_id, version: old_meta.version.as_deref(), md5: old_meta.file_md5.as_deref() };
    let file = find_update(&installed_file, details.summary.version.as_deref(), &details.files, installable).ok_or("no_update")?.clone();

    let dest = d.downloads.join(format!("{}_{}", file.id, safe_file_name(&file.name)));
    let last = AtomicU64::new(0);
    let app2 = app.clone();
    let on_progress = move |p: Progress| {
        let prev = last.load(Ordering::Relaxed);
        if p.downloaded.saturating_sub(prev) >= 256 * 1024 || Some(p.downloaded) == p.total {
            last.store(p.downloaded, Ordering::Relaxed);
            let _ = app2.emit("install-progress", ProgressEvent { stage: "download", downloaded: p.downloaded, total: p.total });
        }
    };
    source.download(&file, &dest, &AtomicBool::new(false), &on_progress).await.map_err(err)?;
    let _ = app.emit("install-progress", ProgressEvent { stage: "install", downloaded: 0, total: None });

    let g = game.clone();
    let archive = dest.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&d.journal);
        let ctx = Ctx { game: &g, staging: &d.staging, backups: &d.backups, journal: &journal, limits: Limits::default() };
        update_files(&ctx, &archive, id, &old_names, slot)
    })
    .await
    .map_err(err)?
    .map_err(|e| match e {
        crate::install::error::InstallError::AlreadyExists(_) => "update_slot_conflict".to_string(),
        other => other.to_string(),
    })?;

    let rows: Vec<(String, Option<String>, i64)> = outcome.files.iter().map(|f| (f.addon_name.clone(), Some(f.sha256.clone()), f.size as i64)).collect();
    store::replace_files(&db.pool, id, &rows).await.map_err(err)?;
    store::save_history(&db.pool, &outcome.batch, id, &old_meta).await.map_err(err)?;
    let new_meta = Meta {
        version: details.summary.version.clone().or(old_meta.version.clone()),
        file_id: Some(file.id.clone()),
        file_md5: file.md5.clone(),
        game_build: game.build_id.clone(),
    };
    store::set_meta(&db.pool, id, &new_meta).await.map_err(err)?;
    store::set_batch(&db.pool, id, &outcome.batch).await.map_err(err)?;
    let _ = std::fs::remove_file(&dest);
    Ok(UpdateResult { name: m.name.clone(), slot, overlaps: outcome.overlaps })
}

/// Load slot in the name of another manager's file: `pak05_dir.vpk`, or `123_pak05_dir.vpk` when switched off.
fn foreign_slot(name: &str) -> Option<u8> {
    let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name).strip_suffix("_dir.vpk")?;
    let digits = base.rsplit_once("_pak").map(|(_, d)| d).or_else(|| base.strip_prefix("pak"))?;
    digits.parse().ok()
}

/// Load slots used by files in the addons folder.
fn used_slots(dir: &std::path::Path) -> Result<Vec<u8>, String> {
    Ok(crate::library::scan::scan_addons(dir).map_err(err)?.iter().filter_map(|a| slot_of(&a.file_name).or_else(|| foreign_slot(&a.file_name))).collect())
}

/// Updates a mod that another manager installed, without leaving the app: downloads the chosen file,
/// swaps the other manager's files for the new ones as one undoable action and keeps the mod's place in
/// the load order and its on/off state. From then on this app manages the mod. When the mod page has
/// several files and `file_id` is not given, the answer is `choose_file`.
#[tauri::command]
pub async fn update_foreign(app: AppHandle, state: State<'_, AppState>, remote_id: String, file_id: Option<String>, name: Option<String>) -> Result<UpdateResult, String> {
    let shown = name.unwrap_or_else(|| remote_id.clone());
    let r = update_foreign_impl(app, &state, remote_id, file_id).await;
    log_update(&state, &shown, None, &r);
    r
}

async fn update_foreign_impl(app: AppHandle, state: &AppState, remote_id: String, file_id: Option<String>) -> Result<UpdateResult, String> {
    let _guard = state.busy.lock().await;
    let game = game(&state)?;
    let d = dirs(&state);
    let db = state.db().await?;
    let source = state.source();
    if store::find_by_remote(&db.pool, source.id(), &remote_id).await.map_err(err)?.is_some_and(|m| m.state == "installed") {
        return Err("already_installed".into());
    }

    let (g0, journal_path, store_path, rid) = (game.clone(), d.journal.clone(), state.app_dir.join("foreign_ids.json"), remote_id.clone());
    let old: Vec<super::library::ForeignAddon> = tokio::task::spawn_blocking(move || super::library::foreign_addons(&g0, &journal_path, &store_path))
        .await
        .map_err(err)??
        .into_iter()
        .filter(|f| f.remote_id.as_deref() == Some(rid.as_str()))
        .collect();
    if old.is_empty() {
        return Err("mod_not_found".into());
    }
    let was_enabled = old.iter().any(|f| f.enabled);
    let old_names: Vec<String> = old.iter().map(|f| f.name.clone()).collect();
    let mut old_slots: Vec<u8> = old_names.iter().filter_map(|n| foreign_slot(n)).collect();
    old_slots.sort_unstable();

    let details = source.get_mod_details(&remote_id).await.map_err(err)?;
    let candidates: Vec<&crate::sources::ModFile> = details.files.iter().filter(|f| installable(&f.name)).collect();
    let file = match &file_id {
        Some(id) => candidates.iter().copied().find(|f| &f.id == id),
        None if candidates.len() == 1 => candidates.first().copied(),
        None => None,
    }
    .ok_or("choose_file")?
    .clone();

    let dest = d.downloads.join(format!("{}_{}", file.id, safe_file_name(&file.name)));
    let last = AtomicU64::new(0);
    let app2 = app.clone();
    let on_progress = move |p: Progress| {
        let prev = last.load(Ordering::Relaxed);
        if p.downloaded.saturating_sub(prev) >= 256 * 1024 || Some(p.downloaded) == p.total {
            last.store(p.downloaded, Ordering::Relaxed);
            let _ = app2.emit("install-progress", ProgressEvent { stage: "download", downloaded: p.downloaded, total: p.total });
        }
    };
    source.download(&file, &dest, &AtomicBool::new(false), &on_progress).await.map_err(err)?;
    let _ = app.emit("install-progress", ProgressEvent { stage: "install", downloaded: 0, total: None });

    let name = details.summary.name.clone();
    let db_id = store::insert_mod(&db.pool, source.id(), &remote_id, &name, details.summary.version.as_deref()).await.map_err(err)?;
    // The new files take the old files' place; when that slot is used by another mod they go after every used slot.
    let used = used_slots(&game.addons_dir)?;
    let first = old_slots.first().copied().filter(|s| !used.contains(s) || old_slots.contains(s));
    let fallback = used.iter().copied().max().map(|m| m.saturating_add(1)).unwrap_or(1);

    let (g, archive, names, d1) = (game.clone(), dest.clone(), old_names.clone(), dirs(&state));
    let outcome = tokio::task::spawn_blocking(move || {
        let journal = Journal::open(&d1.journal);
        let ctx = Ctx { game: &g, staging: &d1.staging, backups: &d1.backups, journal: &journal, limits: Limits::default() };
        let mut tries: Vec<u8> = first.into_iter().collect();
        tries.push(fallback);
        let mut last_err = None;
        for slot in tries {
            match update_files(&ctx, &archive, db_id, &names, slot) {
                Ok(o) => return Ok((o, slot)),
                // a failed attempt puts the old files back; try the next place
                Err(e @ crate::install::error::InstallError::AlreadyExists(_)) => last_err = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or(crate::install::error::InstallError::TooManySlots(crate::install::pak_order::MAX_SLOTS)))
    })
    .await
    .map_err(err)?;
    let (outcome, slot) = match outcome {
        Ok(v) => v,
        Err(e) => {
            let _ = store::delete_mod(&db.pool, db_id).await;
            let _ = std::fs::remove_file(&dest);
            return Err(match e {
                crate::install::error::InstallError::AlreadyExists(_) => "update_slot_conflict".to_string(),
                crate::install::error::InstallError::AlreadyInstalled(n) => format!("duplicate:{n}"),
                other => other.to_string(),
            });
        }
    };

    // `update_files` only sees names, so a mod the other manager had switched off is switched off again here.
    if !was_enabled {
        let (g, d2, files) = (game.clone(), dirs(&state), outcome.files.iter().map(|f| f.addon_name.clone()).collect::<Vec<_>>());
        tokio::task::spawn_blocking(move || {
            let journal = Journal::open(&d2.journal);
            let ctx = Ctx { game: &g, staging: &d2.staging, backups: &d2.backups, journal: &journal, limits: Limits::default() };
            toggle_mod(&ctx, db_id, &files, false)
        })
        .await
        .map_err(err)?
        .map_err(err)?;
    }

    let rows: Vec<(String, Option<String>, i64)> = outcome.files.iter().map(|f| (f.addon_name.clone(), Some(f.sha256.clone()), f.size as i64)).collect();
    store::replace_files(&db.pool, db_id, &rows).await.map_err(err)?;
    store::set_batch(&db.pool, db_id, &outcome.batch).await.map_err(err)?;
    let meta = Meta { version: details.summary.version.clone(), file_id: Some(file.id.clone()), file_md5: file.md5.clone(), game_build: game.build_id.clone() };
    store::set_meta(&db.pool, db_id, &meta).await.map_err(err)?;
    let _ = std::fs::remove_file(&dest);
    Ok(UpdateResult { name, slot, overlaps: outcome.overlaps })
}

/// After the journal rolled an update back: put the old metadata back and keep the file rows whose files are
/// there. An update that took over another manager's mod has no saved metadata: it is that manager's again.
pub(super) async fn restore_after_undo(state: &AppState, mod_id: i64, batch: &str) -> Result<(), String> {
    let db = state.db().await?;
    match store::take_history(&db.pool, batch).await.map_err(err)? {
        Some((id, meta)) => store::set_meta(&db.pool, id, &meta).await.map_err(err)?,
        None => return store::delete_mod(&db.pool, mod_id).await.map_err(err),
    }
    let Some(game) = state.game().ok().flatten() else { return Ok(()) };
    let mut rows = Vec::new();
    for n in store::files_of(&db.pool, mod_id).await.map_err(err)? {
        let size = std::fs::metadata(game.addons_dir.join(&n)).or_else(|_| std::fs::metadata(game.addons_dir.join(format!("{n}{DISABLED_SUFFIX}")))).map(|m| m.len());
        if let Ok(size) = size {
            rows.push((n, None, size as i64));
        }
    }
    if rows.is_empty() {
        return store::delete_mod(&db.pool, mod_id).await.map_err(err);
    }
    store::replace_files(&db.pool, mod_id, &rows).await.map_err(err)
}
