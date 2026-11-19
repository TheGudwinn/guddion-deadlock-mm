//! Queue rows in Postgres. The database is the source of truth for states; the runtime flags
//! (pause/cancel of a transfer in flight) live in `Queue`.
use crate::sources::ModFile;
use serde::Serialize;
use sqlx::PgPool;

pub const QUEUED: &str = "queued";
pub const DOWNLOADING: &str = "downloading";
pub const PAUSED: &str = "paused";
pub const INSTALLING: &str = "installing";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";

/// States in which the mod is still "in the queue" (blocks queuing it a second time).
pub const ACTIVE: [&str; 4] = [QUEUED, DOWNLOADING, PAUSED, INSTALLING];

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub source: String,
    pub remote_id: String,
    pub mod_name: String,
    pub file: ModFile,
    pub version: Option<String>,
    pub state: String,
    pub downloaded: i64,
    pub total: Option<i64>,
    pub error: Option<String>,
    pub slot: Option<i32>,
    pub overlaps: Option<i32>,
}

type Raw = (i64, String, String, String, String, Option<String>, String, i64, Option<i64>, Option<String>, Option<i32>, Option<i32>);

/// `sqlx` only accepts static SQL, so the column list is spliced in at compile time.
macro_rules! select {
    ($tail:literal) => {
        concat!("SELECT id, source, remote_id, mod_name, file_json, version, state, downloaded, total, error, slot, overlap_count FROM download_queue ", $tail)
    };
}

fn to_item(r: Raw) -> Result<Item, sqlx::Error> {
    let file: ModFile = serde_json::from_str(&r.4).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
    Ok(Item { id: r.0, source: r.1, remote_id: r.2, mod_name: r.3, file, version: r.5, state: r.6, downloaded: r.7, total: r.8, error: r.9, slot: r.10, overlaps: r.11 })
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum QueueError {
    #[error("already_queued")]
    AlreadyQueued,
    #[error("database error: {0}")]
    Db(String),
}

impl From<sqlx::Error> for QueueError {
    fn from(e: sqlx::Error) -> Self {
        QueueError::Db(e.to_string())
    }
}

pub async fn enqueue(pool: &PgPool, source: &str, remote_id: &str, mod_name: &str, file: &ModFile, version: Option<&str>) -> Result<i64, QueueError> {
    let dup: Option<(i64,)> = sqlx::query_as("SELECT id FROM download_queue WHERE source = $1 AND remote_id = $2 AND state = ANY($3) LIMIT 1")
        .bind(source)
        .bind(remote_id)
        .bind(ACTIVE.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .fetch_optional(pool)
        .await?;
    if dup.is_some() {
        return Err(QueueError::AlreadyQueued);
    }
    let json = serde_json::to_string(file).map_err(|e| QueueError::Db(e.to_string()))?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO download_queue (source, remote_id, mod_name, file_json, version, state, total) VALUES ($1, $2, $3, $4, $5, 'queued', $6) RETURNING id",
    )
    .bind(source)
    .bind(remote_id)
    .bind(mod_name)
    .bind(json)
    .bind(version)
    .bind(if file.size > 0 { Some(file.size as i64) } else { None })
    .fetch_one(pool)
    .await?;
    Ok(id)
}

pub async fn list(pool: &PgPool) -> Result<Vec<Item>, sqlx::Error> {
    let rows: Vec<Raw> = sqlx::query_as(select!("ORDER BY id")).fetch_all(pool).await?;
    rows.into_iter().map(to_item).collect()
}

pub async fn get(pool: &PgPool, id: i64) -> Result<Option<Item>, sqlx::Error> {
    let row: Option<Raw> = sqlx::query_as(select!("WHERE id = $1")).bind(id).fetch_optional(pool).await?;
    row.map(to_item).transpose()
}

/// Oldest queued item.
pub async fn next_queued(pool: &PgPool) -> Result<Option<Item>, sqlx::Error> {
    let row: Option<Raw> = sqlx::query_as(select!("WHERE state = 'queued' ORDER BY id LIMIT 1")).fetch_optional(pool).await?;
    row.map(to_item).transpose()
}

pub async fn set_state(pool: &PgPool, id: i64, state: &str, error: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE download_queue SET state = $2, error = $3 WHERE id = $1").bind(id).bind(state).bind(error).execute(pool).await?;
    Ok(())
}

pub async fn set_downloaded(pool: &PgPool, id: i64, downloaded: i64, total: Option<i64>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE download_queue SET downloaded = $2, total = COALESCE($3, total) WHERE id = $1").bind(id).bind(downloaded).bind(total).execute(pool).await?;
    Ok(())
}

pub async fn set_done(pool: &PgPool, id: i64, slot: i32, overlaps: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE download_queue SET state = 'done', slot = $2, overlap_count = $3, error = NULL, downloaded = COALESCE(total, downloaded) WHERE id = $1").bind(id).bind(slot).bind(overlaps).execute(pool).await?;
    Ok(())
}

/// Guarded transitions: they return how many rows changed, so callers know whether the move was legal.
pub async fn pause_queued(pool: &PgPool, id: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE download_queue SET state = 'paused' WHERE id = $1 AND state = 'queued'").bind(id).execute(pool).await?.rows_affected())
}

pub async fn resume(pool: &PgPool, id: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE download_queue SET state = 'queued', error = NULL WHERE id = $1 AND state IN ('paused', 'failed')").bind(id).execute(pool).await?.rows_affected())
}

pub async fn cancel_idle(pool: &PgPool, id: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("UPDATE download_queue SET state = 'cancelled' WHERE id = $1 AND state IN ('queued', 'paused', 'failed')").bind(id).execute(pool).await?.rows_affected())
}

/// Removes a finished row (done, failed, cancelled). Active rows are never deleted here.
pub async fn remove_finished(pool: &PgPool, id: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM download_queue WHERE id = $1 AND state IN ('done', 'failed', 'cancelled')").bind(id).execute(pool).await?.rows_affected())
}

pub async fn clear_finished(pool: &PgPool) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM download_queue WHERE state IN ('done', 'cancelled')").execute(pool).await?.rows_affected())
}

/// After a restart nothing is running: interrupted transfers become paused (the user decides when to
/// use the network again), and an interrupted install is reported as failed.
pub async fn recover(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE download_queue SET state = 'paused' WHERE state IN ('queued', 'downloading')").execute(pool).await?;
    sqlx::query("UPDATE download_queue SET state = 'failed', error = 'interrupted' WHERE state = 'installing'").execute(pool).await?;
    Ok(())
}
