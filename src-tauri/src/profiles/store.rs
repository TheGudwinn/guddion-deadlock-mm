use super::ProfileEntry;
use sqlx::PgPool;

pub async fn list(pool: &PgPool) -> Result<Vec<(i64, String, bool)>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, active FROM profiles ORDER BY lower(name), id").fetch_all(pool).await
}

pub async fn name_of(pool: &PgPool, id: i64) -> Result<Option<String>, sqlx::Error> {
    Ok(sqlx::query_as::<_, (String,)>("SELECT name FROM profiles WHERE id = $1").bind(id).fetch_optional(pool).await?.map(|r| r.0))
}

pub async fn entries_of(pool: &PgPool, id: i64) -> Result<Vec<ProfileEntry>, sqlx::Error> {
    let rows: Vec<(String, String, String, Option<String>, bool, i32)> = sqlx::query_as(
        "SELECT source, remote_id, name, version, enabled, position FROM profile_mods WHERE profile_id = $1 ORDER BY position",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| ProfileEntry { source: r.0, remote_id: r.1, name: r.2, version: r.3, enabled: r.4, position: r.5 }).collect())
}

async fn write_entries(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64, entries: &[ProfileEntry]) -> Result<(), sqlx::Error> {
    for e in entries {
        sqlx::query("INSERT INTO profile_mods (profile_id, position, source, remote_id, name, version, enabled) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(id)
            .bind(e.position)
            .bind(&e.source)
            .bind(&e.remote_id)
            .bind(&e.name)
            .bind(&e.version)
            .bind(e.enabled)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

pub async fn create(pool: &PgPool, name: &str, entries: &[ProfileEntry]) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let (id,): (i64,) = sqlx::query_as("INSERT INTO profiles (name) VALUES ($1) RETURNING id").bind(name).fetch_one(&mut *tx).await?;
    write_entries(&mut tx, id, entries).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn replace_entries(pool: &PgPool, id: i64, entries: &[ProfileEntry]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM profile_mods WHERE profile_id = $1").bind(id).execute(&mut *tx).await?;
    write_entries(&mut tx, id, entries).await?;
    tx.commit().await
}

pub async fn rename(pool: &PgPool, id: i64, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE profiles SET name = $2 WHERE id = $1").bind(id).bind(name).execute(pool).await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM profiles WHERE id = $1").bind(id).execute(pool).await?;
    Ok(())
}

/// Make `id` the single active profile, or clear the active mark with `None`.
pub async fn set_active(pool: &PgPool, id: Option<i64>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE profiles SET active = FALSE WHERE active").execute(&mut *tx).await?;
    if let Some(id) = id {
        sqlx::query("UPDATE profiles SET active = TRUE WHERE id = $1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await
}

pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}
