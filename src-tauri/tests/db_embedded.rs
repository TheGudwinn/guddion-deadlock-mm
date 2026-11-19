use deadlock_mm_lib::db::Db;

#[tokio::test]
async fn starts_migrates_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::start(dir.path()).await.unwrap();
    sqlx::query("INSERT INTO profiles (name) VALUES ('default')").execute(&db.pool).await.unwrap();
    db.stop().await.unwrap();

    // Second start reuses data dir and password.
    let db = Db::start(dir.path()).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM profiles").fetch_one(&db.pool).await.unwrap();
    assert_eq!(n, 1);
    db.stop().await.unwrap();
}
