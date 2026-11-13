//! Library records in Postgres. The filesystem is the source of truth for enabled/disabled;
//! these rows only remember what we installed and from where.
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize)]
pub struct ModRow {
    pub id: i64,
    pub source: Option<String>,
    pub remote_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub state: String,
    pub install_batch: Option<String>,
}

pub async fn insert_mod(pool: &PgPool, source: &str, remote_id: &str, name: &str, version: Option<&str>) -> Result<i64, sqlx::Error> {
    let (id,): (i64,) = sqlx::query_as("INSERT INTO mods (source, remote_id, name, version) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(source)
        .bind(remote_id)
        .bind(name)
        .bind(version)
        .fetch_one(pool)
        .await?;
    Ok(id)
}

pub async fn find_by_remote(pool: &PgPool, source: &str, remote_id: &str) -> Result<Option<ModRow>, sqlx::Error> {
    let row: Option<(i64, Option<String>, Option<String>, String, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT id, source, remote_id, name, version, state, install_batch FROM mods WHERE source = $1 AND remote_id = $2",
    )
    .bind(source)
    .bind(remote_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(to_row))
}

type Raw = (i64, Option<String>, Option<String>, String, Option<String>, String, Option<String>);

fn to_row(r: Raw) -> ModRow {
    ModRow { id: r.0, source: r.1, remote_id: r.2, name: r.3, version: r.4, state: r.5, install_batch: r.6 }
}

pub async fn list_mods(pool: &PgPool) -> Result<Vec<ModRow>, sqlx::Error> {
    let rows: Vec<Raw> =
        sqlx::query_as("SELECT id, source, remote_id, name, version, state, install_batch FROM mods ORDER BY installed_at DESC, id DESC")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(to_row).collect())
}

pub async fn get_mod(pool: &PgPool, id: i64) -> Result<Option<ModRow>, sqlx::Error> {
    let row: Option<Raw> = sqlx::query_as("SELECT id, source, remote_id, name, version, state, install_batch FROM mods WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(to_row))
}

pub async fn add_file(pool: &PgPool, mod_id: i64, addon_name: &str, sha256: &str, size: i64) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO mod_files (mod_id, stored_path, sha256, size) VALUES ($1, $2, $3, $4)")
        .bind(mod_id)
        .bind(addon_name)
        .bind(sha256)
        .bind(size)
        .execute(pool)
        .await?;
    Ok(())
}

/// Addon file names (relative to the addons dir) belonging to a mod.
pub async fn files_of(pool: &PgPool, mod_id: i64) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT stored_path FROM mod_files WHERE mod_id = $1 ORDER BY id").bind(mod_id).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn set_batch(pool: &PgPool, id: i64, batch: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mods SET install_batch = $2 WHERE id = $1").bind(id).bind(batch).execute(pool).await?;
    Ok(())
}

pub async fn set_state(pool: &PgPool, id: i64, state: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mods SET state = $2 WHERE id = $1").bind(id).bind(state).execute(pool).await?;
    Ok(())
}

pub async fn delete_mod(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM mods WHERE id = $1").bind(id).execute(pool).await?;
    Ok(())
}

/// (row id, addon file name) of a mod's files in install order.
pub async fn file_rows(pool: &PgPool, mod_id: i64) -> Result<Vec<(i64, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, stored_path FROM mod_files WHERE mod_id = $1 ORDER BY id").bind(mod_id).fetch_all(pool).await
}

pub async fn set_file_name(pool: &PgPool, row_id: i64, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mod_files SET stored_path = $2 WHERE id = $1").bind(row_id).bind(name).execute(pool).await?;
    Ok(())
}

/// What we remember about an installed mod's source file and the game build it was installed on.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub version: Option<String>,
    pub file_id: Option<String>,
    pub file_md5: Option<String>,
    pub game_build: Option<String>,
}

pub async fn install_meta(pool: &PgPool, id: i64) -> Result<Meta, sqlx::Error> {
    let row: Option<(Option<String>, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT version, file_id, file_md5, game_build FROM mods WHERE id = $1").bind(id).fetch_optional(pool).await?;
    Ok(row.map(|(version, file_id, file_md5, game_build)| Meta { version, file_id, file_md5, game_build }).unwrap_or_default())
}

pub async fn set_meta(pool: &PgPool, id: i64, m: &Meta) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mods SET version = $2, file_id = $3, file_md5 = $4, game_build = $5 WHERE id = $1")
        .bind(id)
        .bind(&m.version)
        .bind(&m.file_id)
        .bind(&m.file_md5)
        .bind(&m.game_build)
        .execute(pool)
        .await?;
    Ok(())
}

/// Replaces all file rows of a mod (name, optional sha256, size) in one transaction.
pub async fn replace_files(pool: &PgPool, mod_id: i64, files: &[(String, Option<String>, i64)]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM mod_files WHERE mod_id = $1").bind(mod_id).execute(&mut *tx).await?;
    for (name, sha, size) in files {
        sqlx::query("INSERT INTO mod_files (mod_id, stored_path, sha256, size) VALUES ($1, $2, $3, $4)")
            .bind(mod_id)
            .bind(name)
            .bind(sha)
            .bind(size)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub async fn save_history(pool: &PgPool, batch: &str, mod_id: i64, old: &Meta) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO update_history (batch, mod_id, old_version, old_file_id, old_file_md5, old_game_build) VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (batch) DO NOTHING")
        .bind(batch)
        .bind(mod_id)
        .bind(&old.version)
        .bind(&old.file_id)
        .bind(&old.file_md5)
        .bind(&old.game_build)
        .execute(pool)
        .await?;
    Ok(())
}

/// Returns and removes the metadata saved before an update.
pub async fn take_history(pool: &PgPool, batch: &str) -> Result<Option<(i64, Meta)>, sqlx::Error> {
    let row: Option<(i64, Option<String>, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("DELETE FROM update_history WHERE batch = $1 RETURNING mod_id, old_version, old_file_id, old_file_md5, old_game_build")
            .bind(batch)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id, version, file_id, file_md5, game_build)| (id, Meta { version, file_id, file_md5, game_build })))
}
