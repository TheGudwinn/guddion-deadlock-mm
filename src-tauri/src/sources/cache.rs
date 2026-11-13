//! Offline catalog cache in Postgres (`catalog_cache`) and a wrapper that falls back to it
//! when the network is unavailable.
use super::*;
use sqlx::PgPool;
use std::sync::Arc;

fn db(e: sqlx::Error) -> SourceError {
    SourceError::Db(e.to_string())
}

pub async fn put<T: Serialize>(pool: &PgPool, source: &str, key: &str, value: &T) -> Result<(), SourceError> {
    let json = serde_json::to_value(value).map_err(|e| SourceError::Parse(e.to_string()))?;
    sqlx::query(
        "INSERT INTO catalog_cache (source, remote_id, json, fetched_at) VALUES ($1, $2, $3, now())
         ON CONFLICT (source, remote_id) DO UPDATE SET json = EXCLUDED.json, fetched_at = now()",
    )
    .bind(source)
    .bind(key)
    .bind(json)
    .execute(pool)
    .await
    .map_err(db)?;
    Ok(())
}

/// Returns the cached value and the unix time it was fetched.
pub async fn get<T: serde::de::DeserializeOwned>(pool: &PgPool, source: &str, key: &str) -> Result<Option<(T, i64)>, SourceError> {
    let row: Option<(serde_json::Value, i64)> = sqlx::query_as(
        "SELECT json, EXTRACT(EPOCH FROM fetched_at)::BIGINT FROM catalog_cache WHERE source = $1 AND remote_id = $2",
    )
    .bind(source)
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(db)?;
    match row {
        None => Ok(None),
        Some((v, ts)) => Ok(Some((serde_json::from_value(v).map_err(|e| SourceError::Parse(e.to_string()))?, ts))),
    }
}

fn search_key(q: &SearchQuery) -> String {
    let t = q.text.as_deref().map(str::trim).unwrap_or("").to_lowercase();
    format!("search|{t}|{:?}|{:?}|{}|{}", q.category, q.sort, q.page.max(1), q.per_page)
}

/// Wraps any source: successful responses are cached, and when the network fails the last cached
/// response is returned with `cached_at` set so the UI can show "offline, data from <date>".
pub struct CachedSource {
    inner: Arc<dyn ModSource>,
    pool: PgPool,
}

impl CachedSource {
    pub fn new(inner: Arc<dyn ModSource>, pool: PgPool) -> Self {
        Self { inner, pool }
    }
}

#[async_trait]
impl ModSource for CachedSource {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, SourceError> {
        let key = search_key(q);
        match self.inner.search(q).await {
            Ok(p) => {
                let _ = put(&self.pool, self.id(), &key, &p).await; // cache failures must not break browsing
                Ok(p)
            }
            Err(SourceError::Net(e)) => match get::<SearchPage>(&self.pool, self.id(), &key).await? {
                Some((mut p, ts)) => {
                    p.cached_at = Some(ts);
                    Ok(p)
                }
                None => Err(SourceError::Net(e)),
            },
            Err(e) => Err(e),
        }
    }

    async fn get_mod_details(&self, id: &str) -> Result<ModDetails, SourceError> {
        let key = format!("mod|{id}");
        match self.inner.get_mod_details(id).await {
            Ok(d) => {
                let _ = put(&self.pool, self.id(), &key, &d).await;
                Ok(d)
            }
            Err(SourceError::Net(e)) => match get::<ModDetails>(&self.pool, self.id(), &key).await? {
                Some((mut d, ts)) => {
                    d.cached_at = Some(ts);
                    Ok(d)
                }
                None => Err(SourceError::Net(e)),
            },
            Err(e) => Err(e),
        }
    }

    async fn categories(&self, parent: Option<i64>) -> Result<Vec<Category>, SourceError> {
        let key = format!("cats|{parent:?}");
        match self.inner.categories(parent).await {
            Ok(c) => {
                let _ = put(&self.pool, self.id(), &key, &c).await;
                Ok(c)
            }
            Err(SourceError::Net(e)) => match get::<Vec<Category>>(&self.pool, self.id(), &key).await? {
                Some((c, _)) => Ok(c),
                None => Err(SourceError::Net(e)),
            },
            Err(e) => Err(e),
        }
    }

    async fn download(
        &self,
        file: &ModFile,
        dest: &Path,
        pause: &AtomicBool,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(), SourceError> {
        self.inner.download(file, dest, pause, on_progress).await
    }
}
