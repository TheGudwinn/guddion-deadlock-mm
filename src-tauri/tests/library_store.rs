use deadlock_mm_lib::db::Db;
use deadlock_mm_lib::library::store;

#[tokio::test]
async fn store_roundtrip_matches_what_the_commands_rely_on() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::start(dir.path()).await.unwrap();
    let p = &db.pool;

    let id = store::insert_mod(p, "gamebanana", "650634", "QOL Lock", Some("4.0.5")).await.unwrap();
    store::add_file(p, id, "ddmm1_pak01_dir.vpk", "abc", 10).await.unwrap();
    store::add_file(p, id, "ddmm1_pak02_dir.vpk", "def", 20).await.unwrap();
    store::set_batch(p, id, "install-1-123").await.unwrap();

    assert_eq!(store::files_of(p, id).await.unwrap(), ["ddmm1_pak01_dir.vpk", "ddmm1_pak02_dir.vpk"]);
    let row = store::find_by_remote(p, "gamebanana", "650634").await.unwrap().unwrap();
    assert_eq!((row.id, row.state.as_str(), row.install_batch.as_deref()), (id, "installed", Some("install-1-123")));

    // The same remote mod cannot be recorded twice...
    assert!(store::insert_mod(p, "gamebanana", "650634", "dup", None).await.is_err());

    // ...but after uninstall (state removed) the command deletes the row and a clean re-install works.
    store::set_state(p, id, "removed").await.unwrap();
    assert_eq!(store::find_by_remote(p, "gamebanana", "650634").await.unwrap().unwrap().state, "removed");
    store::delete_mod(p, id).await.unwrap();
    assert!(store::files_of(p, id).await.unwrap().is_empty(), "files cascade with the mod");
    let again = store::insert_mod(p, "gamebanana", "650634", "QOL Lock", None).await.unwrap();
    assert_ne!(again, id);

    assert_eq!(store::list_mods(p).await.unwrap().len(), 1);
    db.stop().await.unwrap();
}

#[tokio::test]
async fn update_metadata_files_and_history_roundtrip() {
    use deadlock_mm_lib::library::store::Meta;
    let dir = tempfile::tempdir().unwrap();
    let db = Db::start(dir.path()).await.unwrap();
    let p = &db.pool;

    let id = store::insert_mod(p, "gamebanana", "1", "Mod", Some("1.0")).await.unwrap();
    // A freshly inserted row has no file id (like mods installed before the migration).
    assert!(store::install_meta(p, id).await.unwrap().file_id.is_none());

    let v1 = Meta { version: Some("1.0".into()), file_id: Some("100".into()), file_md5: Some("aa".into()), game_build: Some("500".into()) };
    store::set_meta(p, id, &v1).await.unwrap();
    store::add_file(p, id, "ddmm1_pak01_dir.vpk", "old", 5).await.unwrap();

    // Update: history is kept under the batch, files and metadata are replaced.
    store::save_history(p, "update-1-9", id, &v1).await.unwrap();
    let v2 = Meta { version: Some("2.0".into()), file_id: Some("200".into()), file_md5: None, game_build: Some("600".into()) };
    store::set_meta(p, id, &v2).await.unwrap();
    store::replace_files(p, id, &[("ddmm1_pak01_dir.vpk".into(), Some("new".into()), 7), ("ddmm1_pak02_dir.vpk".into(), None, 8)]).await.unwrap();
    assert_eq!(store::files_of(p, id).await.unwrap().len(), 2);
    assert_eq!(store::install_meta(p, id).await.unwrap().file_id.as_deref(), Some("200"));

    // Undo: the saved metadata comes back exactly once.
    let (mod_id, old) = store::take_history(p, "update-1-9").await.unwrap().unwrap();
    assert_eq!(mod_id, id);
    store::set_meta(p, id, &old).await.unwrap();
    let back = store::install_meta(p, id).await.unwrap();
    assert_eq!((back.file_id.as_deref(), back.game_build.as_deref(), back.version.as_deref()), (Some("100"), Some("500"), Some("1.0")));
    assert!(store::take_history(p, "update-1-9").await.unwrap().is_none());

    // The file list from the update is still what is stored (undo rebuilds it from disk in the command).
    assert_eq!(store::files_of(p, id).await.unwrap().len(), 2);
    db.stop().await.unwrap();
}
