//! Sanity check of the update heuristic against real GameBanana pages (network required):
//! `cargo test --test live_mod_versions -- --ignored --nocapture`
use deadlock_mm_lib::library::updates::{find_update, installable, InstalledFile};
use deadlock_mm_lib::net::{config::NetConfig, NetService};
use deadlock_mm_lib::sources::{gamebanana::GameBanana, ModSource};
use std::sync::Arc;

#[tokio::test]
#[ignore = "talks to the real GameBanana"]
async fn the_file_we_just_installed_is_never_reported_as_an_update() {
    let gb = GameBanana::new(Arc::new(NetService::new(NetConfig::default()).unwrap()));
    // Popular mods from the catalog, so the sample is real and varied.
    let page = gb.search(&deadlock_mm_lib::sources::SearchQuery { text: None, category: None, sort: Default::default(), page: 1, per_page: 8 }).await.unwrap();
    let mut checked = 0;
    for m in page.items {
        let d = gb.get_mod_details(&m.id).await.unwrap();
        let Some(file) = d.files.iter().find(|f| installable(&f.name)) else { continue };
        let installed = InstalledFile { file_id: &file.id, version: d.summary.version.as_deref(), md5: file.md5.as_deref() };
        let hit = find_update(&installed, d.summary.version.as_deref(), &d.files, installable);
        println!("{:<45} files={} installed={} -> {:?}", m.name, d.files.len(), file.id, hit.map(|f| f.id.clone()));
        // Installing the first installable file and checking right away must say "up to date"
        // unless the page lists a newer sibling AND a version we could not have seen (impossible here).
        assert!(hit.is_none(), "false positive for {}", m.name);

        // An older file id with an older version must be flagged.
        let old = InstalledFile { file_id: "1", version: Some("0.0.0-old"), md5: None };
        assert!(find_update(&old, d.summary.version.as_deref(), &d.files, installable).is_some());
        checked += 1;
    }
    assert!(checked >= 3, "too few mods with installable files: {checked}");
}
