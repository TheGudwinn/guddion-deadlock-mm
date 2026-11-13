//! Download queue: persistent items, one transfer at a time, pause / resume / cancel.
//!
//! The worker is written against the `Host` trait so the state machine can be tested without
//! Tauri or the network; the real host lives in `commands::queue`.
pub mod store;

use crate::error::NetError;
use crate::net::download::{part_path, Progress};
use crate::sources::{ModFile, SourceError};
use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use store::Item;
use tokio::sync::Notify;

/// File name that is safe to create under our downloads folder.
pub fn safe_file_name(name: &str) -> String {
    let base = Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or("mod.bin");
    base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

pub fn dest_path(downloads: &Path, file: &ModFile) -> PathBuf {
    downloads.join(format!("{}_{}", file.id, safe_file_name(&file.name)))
}

/// What the host (the app) does for the worker.
#[async_trait]
pub trait Host: Send + Sync {
    fn downloads_dir(&self) -> PathBuf;
    async fn download(&self, file: &ModFile, dest: &Path, pause: &AtomicBool, on_progress: &(dyn Fn(Progress) + Send + Sync)) -> Result<(), SourceError>;
    /// Installs the downloaded archive; returns the slot it went to and how many installed addons it overlaps with.
    async fn install(&self, item: &Item, archive: &Path) -> Result<(u8, u32), String>;
    /// Something changed in the list (states).
    fn changed(&self);
    /// Throttle-worthy progress of a running transfer.
    fn progress(&self, id: i64, downloaded: u64, total: Option<u64>);
}

#[derive(Default)]
pub struct Flags {
    pub pause: AtomicBool,
    pub cancel: AtomicBool,
}

/// Runtime part of the queue (what the database cannot hold).
#[derive(Default)]
pub struct Queue {
    flags: Mutex<HashMap<i64, Arc<Flags>>>,
    progress: Mutex<HashMap<i64, (u64, Option<u64>)>>,
    pub notify: Notify,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh_flags(&self, id: i64) -> Arc<Flags> {
        let f = Arc::new(Flags::default());
        self.flags.lock().unwrap().insert(id, f.clone());
        f
    }

    fn flags_of(&self, id: i64) -> Option<Arc<Flags>> {
        self.flags.lock().unwrap().get(&id).cloned()
    }

    fn forget(&self, id: i64) {
        self.flags.lock().unwrap().remove(&id);
        self.progress.lock().unwrap().remove(&id);
    }

    pub fn set_progress(&self, id: i64, downloaded: u64, total: Option<u64>) {
        self.progress.lock().unwrap().insert(id, (downloaded, total));
    }

    pub fn progress_of(&self, id: i64) -> Option<(u64, Option<u64>)> {
        self.progress.lock().unwrap().get(&id).copied()
    }

    // ---- user actions ----

    /// Adds an item and wakes the worker. Fails with `AlreadyQueued` while the same mod is still active.
    pub async fn enqueue(&self, pool: &PgPool, source: &str, remote_id: &str, mod_name: &str, file: &ModFile, version: Option<&str>) -> Result<i64, store::QueueError> {
        let id = store::enqueue(pool, source, remote_id, mod_name, file, version).await?;
        self.notify.notify_one();
        Ok(id)
    }

    /// Queued -> paused right away; a running download is asked to stop (the worker marks it paused).
    pub async fn pause(&self, pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
        if store::pause_queued(pool, id).await? > 0 {
            return Ok(());
        }
        if let Some(item) = store::get(pool, id).await? {
            if item.state == store::DOWNLOADING {
                if let Some(f) = self.flags_of(id) {
                    f.pause.store(true, Ordering::Relaxed);
                }
            }
        }
        Ok(())
    }

    /// Paused or failed -> queued again (a partial file is continued with an HTTP Range request).
    pub async fn resume(&self, pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
        if store::resume(pool, id).await? > 0 {
            self.notify.notify_one();
        }
        Ok(())
    }

    /// Stops the item for good and deletes its partial download.
    pub async fn cancel(&self, pool: &PgPool, downloads: &Path, id: i64) -> Result<(), sqlx::Error> {
        let Some(item) = store::get(pool, id).await? else { return Ok(()) };
        if item.state == store::DOWNLOADING {
            if let Some(f) = self.flags_of(id) {
                f.cancel.store(true, Ordering::Relaxed);
                f.pause.store(true, Ordering::Relaxed);
            }
            return Ok(()); // the worker finishes the cancellation
        }
        if store::cancel_idle(pool, id).await? > 0 {
            remove_files(downloads, &item.file);
        }
        Ok(())
    }
}

fn remove_files(downloads: &Path, file: &ModFile) {
    let dest = dest_path(downloads, file);
    let _ = std::fs::remove_file(part_path(&dest));
    let _ = std::fs::remove_file(&dest);
}

/// Processes queued items one after another until the process ends.
pub async fn run(pool: PgPool, q: Arc<Queue>, host: Arc<dyn Host>) {
    loop {
        match store::next_queued(&pool).await {
            Ok(Some(item)) => process_one(&pool, &q, host.as_ref(), item).await,
            Ok(None) => q.notify.notified().await,
            Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
        }
    }
}

pub async fn process_one(pool: &PgPool, q: &Queue, host: &dyn Host, item: Item) {
    let id = item.id;
    let flags = q.fresh_flags(id);
    let downloads = host.downloads_dir();
    let dest = dest_path(&downloads, &item.file);

    let _ = store::set_state(pool, id, store::DOWNLOADING, None).await;
    host.changed();

    let result = host
        .download(&item.file, &dest, &flags.pause, &|p: Progress| {
            q.set_progress(id, p.downloaded, p.total);
            host.progress(id, p.downloaded, p.total);
        })
        .await;

    match result {
        Ok(()) => {}
        Err(SourceError::Net(NetError::Paused)) => {
            if flags.cancel.load(Ordering::Relaxed) {
                remove_files(&downloads, &item.file);
                let _ = store::set_state(pool, id, store::CANCELLED, None).await;
            } else {
                if let Some((d, t)) = q.progress_of(id) {
                    let _ = store::set_downloaded(pool, id, d as i64, t.map(|t| t as i64)).await;
                }
                let _ = store::set_state(pool, id, store::PAUSED, None).await;
            }
            q.forget(id);
            host.changed();
            return;
        }
        Err(e) => {
            let _ = store::set_state(pool, id, store::FAILED, Some(&e.to_string())).await;
            q.forget(id);
            host.changed();
            return;
        }
    }

    let _ = store::set_state(pool, id, store::INSTALLING, None).await;
    host.changed();
    match host.install(&item, &dest).await {
        Ok((slot, overlaps)) => {
            let _ = store::set_done(pool, id, slot as i32, overlaps as i32).await;
            let _ = std::fs::remove_file(&dest);
        }
        Err(e) => {
            let _ = store::set_state(pool, id, store::FAILED, Some(&e)).await;
        }
    }
    q.forget(id);
    host.changed();
}
