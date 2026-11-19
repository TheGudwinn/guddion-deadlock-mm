use deadlock_mm_lib::db::Db;
use deadlock_mm_lib::profiles::store;
use deadlock_mm_lib::profiles::ProfileEntry;

fn e(id: &str, enabled: bool, pos: i32) -> ProfileEntry {
    ProfileEntry { source: "gamebanana".into(), remote_id: id.into(), name: format!("mod {id}"), version: Some("1".into()), enabled, position: pos }
}

#[tokio::test]
async fn profiles_roundtrip_unique_names_and_single_active() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::start(dir.path()).await.unwrap();
    let p = &db.pool;

    let a = store::create(p, "Ranked", &[e("1", true, 0), e("2", false, 1)]).await.unwrap();
    let b = store::create(p, "Fun", &[]).await.unwrap();
    assert_eq!(store::entries_of(p, a).await.unwrap(), [e("1", true, 0), e("2", false, 1)]);

    // Duplicate names are rejected (case-sensitive unique) and reported as such.
    let dup = store::create(p, "Ranked", &[]).await.unwrap_err();
    assert!(store::is_unique_violation(&dup));
    // A failed create leaves no half-written profile behind.
    assert_eq!(store::list(p).await.unwrap().len(), 2);

    // Only one profile can be active.
    store::set_active(p, Some(a)).await.unwrap();
    store::set_active(p, Some(b)).await.unwrap();
    let active: Vec<_> = store::list(p).await.unwrap().into_iter().filter(|r| r.2).map(|r| r.0).collect();
    assert_eq!(active, [b]);
    store::set_active(p, None).await.unwrap();
    assert!(store::list(p).await.unwrap().iter().all(|r| !r.2));

    // Update replaces entries atomically; rename and delete cascade.
    store::replace_entries(p, a, &[e("9", true, 0)]).await.unwrap();
    assert_eq!(store::entries_of(p, a).await.unwrap(), [e("9", true, 0)]);
    store::rename(p, a, "Ranked v2").await.unwrap();
    assert_eq!(store::name_of(p, a).await.unwrap().as_deref(), Some("Ranked v2"));
    store::delete(p, a).await.unwrap();
    assert!(store::entries_of(p, a).await.unwrap().is_empty());

    db.stop().await.unwrap();
}
