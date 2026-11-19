use async_trait::async_trait;
use deadlock_mm_lib::db::Db;
use deadlock_mm_lib::error::NetError;
use deadlock_mm_lib::net::download::Progress;
use deadlock_mm_lib::sources::cache::CachedSource;
use deadlock_mm_lib::sources::{Category, ModDetails, ModFile, ModSource, ModSummary, SearchPage, SearchQuery, SourceError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Works while `online` is true, then behaves like a blocked network.
struct Flaky {
    online: Arc<AtomicBool>,
}

#[async_trait]
impl ModSource for Flaky {
    fn id(&self) -> &'static str {
        "fake"
    }
    async fn search(&self, q: &SearchQuery) -> Result<SearchPage, SourceError> {
        if !self.online.load(Ordering::SeqCst) {
            return Err(SourceError::Net(NetError::AllMirrorsFailed("blocked".into())));
        }
        let item = ModSummary {
            source: "fake".into(),
            id: "1".into(),
            name: format!("mod for {:?}", q.text),
            author: None,
            category: None,
            thumbnail: None,
            version: None,
            likes: 1,
            views: 2,
            updated_at: 3,
            page_url: String::new(),
        };
        Ok(SearchPage { items: vec![item], total: 1, page: 1, has_more: false, cached_at: None })
    }
    async fn get_mod_details(&self, _: &str) -> Result<ModDetails, SourceError> {
        Err(SourceError::NotFound)
    }
    async fn categories(&self, _: Option<i64>) -> Result<Vec<Category>, SourceError> {
        Ok(vec![])
    }
    async fn download(&self, _: &ModFile, _: &Path, _: &AtomicBool, _: &(dyn Fn(Progress) + Send + Sync)) -> Result<(), SourceError> {
        Ok(())
    }
}

#[tokio::test]
async fn serves_cached_catalog_when_network_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::start(dir.path()).await.unwrap();
    let online = Arc::new(AtomicBool::new(true));
    let src = CachedSource::new(Arc::new(Flaky { online: online.clone() }), db.pool.clone());
    let q = SearchQuery { text: Some("hud".into()), page: 1, per_page: 20, ..Default::default() };

    // Online: fresh data, not marked as cached.
    let fresh = src.search(&q).await.unwrap();
    assert!(fresh.cached_at.is_none());

    // Offline: same query comes from the database with a timestamp.
    online.store(false, Ordering::SeqCst);
    let cached = src.search(&q).await.unwrap();
    assert_eq!(cached.items, fresh.items);
    assert!(cached.cached_at.is_some_and(|t| t > 1_700_000_000));

    // Offline and never seen before: a real error, so the UI can offer proxy/mirror help.
    let other = SearchQuery { text: Some("never searched".into()), page: 1, per_page: 20, ..Default::default() };
    assert!(matches!(src.search(&other).await, Err(SourceError::Net(_))));

    db.stop().await.unwrap();
}
