//! Download queue commands and the app-side `Host` for the queue worker.
use super::library::perform_install;
use super::{err, AppState};
use crate::net::download::{part_path, Progress};
use crate::queue::store::{self, Item, QueueError};
use crate::queue::{dest_path, Host};
use crate::sources::{ModFile, SourceError};
use async_trait::async_trait;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};

pub struct TauriHost {
    app: AppHandle,
    last_emit_ms: AtomicU64,
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app, last_emit_ms: AtomicU64::new(0) }
    }

    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    id: i64,
    downloaded: u64,
    total: Option<u64>,
}

#[async_trait]
impl Host for TauriHost {
    fn downloads_dir(&self) -> PathBuf {
        self.state().app_dir.join("downloads")
    }

    async fn download(&self, file: &ModFile, dest: &Path, pause: &AtomicBool, on_progress: &(dyn Fn(Progress) + Send + Sync)) -> Result<(), SourceError> {
        let source = self.state().source();
        source.download(file, dest, pause, on_progress).await
    }

    async fn install(&self, item: &Item, archive: &Path) -> Result<(u8, u32), String> {
        let state = self.state();
        let r = perform_install(&state, &item.remote_id, &item.mod_name, item.version.clone(), &item.file, archive).await?;
        let files: std::collections::HashSet<&String> = r.overlaps.iter().map(|o| &o.with_addon).collect();
        Ok((r.slot, files.len() as u32))
    }

    fn log(&self, item: &Item, state: &str, detail: Option<&str>) {
        crate::history::push(
            &self.state().app_dir,
            crate::history::Entry { at: crate::history::now(), kind: "install".into(), name: item.mod_name.clone(), file: Some(item.file.name.clone()), state: state.into(), detail: detail.map(str::to_string) },
        );
    }

    fn changed(&self) {
        let _ = self.app.emit("queue-changed", ());
    }

    fn progress(&self, id: i64, downloaded: u64, total: Option<u64>) {
        // At most ~5 events per second; chunks arrive far more often.
        let now = now_ms();
        let last = self.last_emit_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= 200 || Some(downloaded) == total {
            self.last_emit_ms.store(now, Ordering::Relaxed);
            let _ = self.app.emit("queue-progress", ProgressPayload { id, downloaded, total });
        }
    }
}

/// Everything that was installed or updated, newest first. Kept between runs.
#[tauri::command]
pub fn list_history(state: State<'_, AppState>) -> Vec<crate::history::Entry> {
    let mut all = crate::history::list(&state.app_dir);
    all.reverse();
    all
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) {
    crate::history::clear(&state.app_dir);
}

#[derive(Serialize)]
pub struct QueueRow {
    pub id: i64,
    pub remote_id: String,
    pub name: String,
    pub file_name: String,
    pub state: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub slot: Option<i32>,
    pub overlaps: Option<i32>,
}

fn notify(app: &AppHandle) {
    let _ = app.emit("queue-changed", ());
}

/// Checks that can be answered immediately, then puts the file in the queue. Returns the queue id.
#[tauri::command]
pub async fn enqueue_install(app: AppHandle, state: State<'_, AppState>, mod_id: String, file_id: Option<String>) -> Result<i64, String> {
    let source = state.source();
    let details = source.get_mod_details(&mod_id).await.map_err(err)?;
    let file = match &file_id {
        Some(id) => details.files.iter().find(|f| &f.id == id),
        None => details.files.first(),
    }
    .ok_or("no_files")?
    .clone();
    if !crate::library::updates::installable(&file.name) {
        return Err(format!("unsupported_format:{}", file.name));
    }

    let db = state.db().await?;
    if let Some(existing) = crate::library::store::find_by_remote(&db.pool, source.id(), &mod_id).await.map_err(err)? {
        if existing.state == "installed" {
            return Err("already_installed".into());
        }
    }
    let id = state
        .queue
        .enqueue(&db.pool, source.id(), &mod_id, &details.summary.name, &file, details.summary.version.as_deref())
        .await
        .map_err(|e| match e {
            QueueError::AlreadyQueued => "already_queued".to_string(),
            other => other.to_string(),
        })?;
    notify(&app);
    Ok(id)
}

#[tauri::command]
pub async fn list_queue(state: State<'_, AppState>) -> Result<Vec<QueueRow>, String> {
    let db = state.db().await?;
    let downloads = state.app_dir.join("downloads");
    let items = store::list(&db.pool).await.map_err(err)?;
    Ok(items
        .into_iter()
        .map(|it| {
            let (downloaded, total) = match state.queue.progress_of(it.id) {
                Some((d, t)) => (d, t.or(Some(it.file.size).filter(|s| *s > 0))),
                None => {
                    // Paused or waiting: the partial file is the truth about what is already here.
                    let part = std::fs::metadata(part_path(&dest_path(&downloads, &it.file))).map(|m| m.len()).unwrap_or(0);
                    (part.max(it.downloaded as u64), it.total.map(|t| t as u64))
                }
            };
            QueueRow {
                id: it.id,
                remote_id: it.remote_id,
                name: it.mod_name,
                file_name: it.file.name,
                state: it.state,
                downloaded,
                total,
                error: it.error,
                slot: it.slot,
                overlaps: it.overlaps,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn pause_download(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.queue.pause(&state.db().await?.pool, id).await.map_err(err)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn resume_download(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.queue.resume(&state.db().await?.pool, id).await.map_err(err)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn cancel_download(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<(), String> {
    state.queue.cancel(&state.db().await?.pool, &state.app_dir.join("downloads"), id).await.map_err(err)?;
    notify(&app);
    Ok(())
}

/// Dismisses a finished row (done, failed or cancelled). Partial files of failed items are removed.
#[tauri::command]
pub async fn remove_download(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let db = state.db().await?;
    if let Some(item) = store::get(&db.pool, id).await.map_err(err)? {
        if store::remove_finished(&db.pool, id).await.map_err(err)? > 0 {
            let dest = dest_path(&state.app_dir.join("downloads"), &item.file);
            let _ = std::fs::remove_file(part_path(&dest));
            let _ = std::fs::remove_file(&dest);
        }
    }
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn clear_finished_downloads(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    store::clear_finished(&state.db().await?.pool).await.map_err(err)?;
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn pause_all_downloads(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db().await?;
    for it in store::list(&db.pool).await.map_err(err)? {
        if it.state == store::QUEUED || it.state == store::DOWNLOADING {
            state.queue.pause(&db.pool, it.id).await.map_err(err)?;
        }
    }
    notify(&app);
    Ok(())
}

#[tauri::command]
pub async fn resume_all_downloads(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let db = state.db().await?;
    for it in store::list(&db.pool).await.map_err(err)? {
        if it.state == store::PAUSED {
            state.queue.resume(&db.pool, it.id).await.map_err(err)?;
        }
    }
    notify(&app);
    Ok(())
}
