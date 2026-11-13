//! Updating installed mods: check for newer files, replace files atomically, undo.
use super::library::{dirs, game, installed_mods, safe_file_name};
use super::{err, AppState};
use crate::install::extract::Limits;
use crate::install::installer::{update_files, Ctx};
use crate::install::journal::Journal;
use crate::install::pak_order::{is_ours, mod_id_of, slot_of, DISABLED_SUFFIX};
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

/// After the journal rolled an update back: put the old metadata back and rebuild the file rows
/// from what is on disk now.
pub(super) async fn restore_after_undo(state: &AppState, mod_id: i64, batch: &str) -> Result<(), String> {
    let db = state.db().await?;
    if let Some((id, meta)) = store::take_history(&db.pool, batch).await.map_err(err)? {
        store::set_meta(&db.pool, id, &meta).await.map_err(err)?;
    }
    let Some(game) = state.game().ok().flatten() else { return Ok(()) };
    let mut names: Vec<String> = Vec::new();
    for e in std::fs::read_dir(&game.addons_dir).map_err(err)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_ours(&name) {
            continue;
        }
        let base = name.strip_suffix(DISABLED_SUFFIX).unwrap_or(&name).to_string();
        if mod_id_of(&base) == Some(mod_id) {
            names.push(base);
        }
    }
    names.sort_by_key(|n| slot_of(n).unwrap_or(u8::MAX));
    let mut rows = Vec::new();
    for n in names {
        let size = std::fs::metadata(game.addons_dir.join(&n)).or_else(|_| std::fs::metadata(game.addons_dir.join(format!("{n}{DISABLED_SUFFIX}")))).map(|m| m.len()).unwrap_or(0);
        rows.push((n, None, size as i64));
    }
    store::replace_files(&db.pool, mod_id, &rows).await.map_err(err)
}
