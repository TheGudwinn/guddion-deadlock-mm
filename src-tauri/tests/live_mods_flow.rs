//! Live end-to-end check against the real GameBanana, installing into a TEMPORARY fake game folder
//! whose gameinfo.gi is a read-only copy of the real one. The real game folder is never written.
//! Run: cargo test --test live_mods_flow -- --ignored --nocapture
use deadlock_mm_lib::install::extract::Limits;
use deadlock_mm_lib::install::installer::{install_mod, toggle_mod, uninstall_files, Ctx};
use deadlock_mm_lib::install::journal::Journal;
use deadlock_mm_lib::library::scan::scan_addons;
use deadlock_mm_lib::net::config::NetConfig;
use deadlock_mm_lib::net::NetService;
use deadlock_mm_lib::sources::gamebanana::GameBanana;
use deadlock_mm_lib::sources::{ModSource, SearchQuery, Sort};
use deadlock_mm_lib::steam::locate::validate_game_dir;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const REAL_GAMEINFO: &str = r"D:\SteamLibrary\steamapps\common\Deadlock\game\citadel\gameinfo.gi";

#[tokio::test]
#[ignore]
async fn real_mods_through_the_whole_pipeline() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("Deadlock");
    let citadel = root.join("game/citadel");
    std::fs::create_dir_all(&citadel).unwrap();
    let original = std::fs::read(REAL_GAMEINFO).expect("real gameinfo.gi (read-only copy)");
    std::fs::write(citadel.join("gameinfo.gi"), &original).unwrap();
    let game = validate_game_dir(&root).unwrap();

    let gb = GameBanana::new(Arc::new(NetService::new(NetConfig::default()).unwrap()));
    let page = gb.search(&SearchQuery { sort: Sort::MostLiked, page: 1, per_page: 20, ..Default::default() }).await.unwrap();

    let journal = Journal::open(tmp.path().join("journal.jsonl"));
    let (staging, backups) = (tmp.path().join("staging"), tmp.path().join("backups"));
    let ctx = Ctx { game: &game, staging: &staging, backups: &backups, journal: &journal, limits: Limits::default() };

    let mut ok = Vec::new();
    let mut failed = Vec::new();
    let mut next_id = 1i64;
    let mut slot = 1u8;

    for m in page.items.iter().take(14) {
        let d = match gb.get_mod_details(&m.id).await {
            Ok(d) => d,
            Err(e) => {
                failed.push(format!("{}: details: {e}", m.name));
                continue;
            }
        };
        // Smallest installable file under 40 MB keeps the run fast.
        let Some(f) = d
            .files
            .iter()
            .filter(|f| {
                let n = f.name.to_lowercase();
                (n.ends_with(".zip") || n.ends_with(".vpk") || n.ends_with(".rar") || n.ends_with(".7z")) && f.size < 40 << 20
            })
            .min_by_key(|f| f.size)
        else {
            failed.push(format!("{}: no zip/7z/rar/vpk under 40MB (files: {:?})", m.name, d.files.iter().map(|f| f.name.clone()).collect::<Vec<_>>()));
            continue;
        };

        let dest = tmp.path().join(format!("dl_{}_{}", f.id, f.name.replace(['/', '\\'], "_")));
        if let Err(e) = gb.download(f, &dest, &AtomicBool::new(false), &|_| {}).await {
            failed.push(format!("{}: download {}: {e}", m.name, f.name));
            continue;
        }
        let md5_checked = f.md5.is_some();
        match install_mod(&ctx, &dest, next_id, slot) {
            Ok(out) => {
                for file in &out.files {
                    assert!(file.path.is_file());
                    assert!(file.entries.len() > 0, "VPK index should list files");
                }
                println!(
                    "OK   {} | {} | md5 {} | {} vpk, {} entries, overlaps {}",
                    m.name,
                    f.name,
                    if md5_checked { "verified" } else { "n/a" },
                    out.files.len(),
                    out.files.iter().map(|x| x.entries.len()).sum::<usize>(),
                    out.overlaps.len()
                );
                slot += out.files.len() as u8;
                ok.push((next_id, out.files.iter().map(|x| x.addon_name.clone()).collect::<Vec<_>>()));
                next_id += 1;
            }
            Err(e) => failed.push(format!("{}: install {}: {e}", m.name, f.name)),
        }
    }

    println!("\n--- installed: {} / failed: {} ---", ok.len(), failed.len());
    for f in &failed {
        println!("FAIL {f}");
    }
    assert!(!ok.is_empty(), "at least one real mod must install");

    // gameinfo.gi: real file already had the addons line, so it must be byte-identical.
    assert_eq!(std::fs::read(&game.gameinfo).unwrap(), original, "installer must not touch an already-patched gameinfo.gi");
    assert_eq!(scan_addons(&game.addons_dir).unwrap().len(), ok.iter().map(|(_, f)| f.len()).sum::<usize>());

    // Disable everything, then re-enable.
    for (id, files) in &ok {
        toggle_mod(&ctx, *id, files, false).unwrap();
    }
    assert!(std::fs::read_dir(&game.addons_dir).unwrap().all(|e| e.unwrap().file_name().to_string_lossy().ends_with(".disabled")));
    for (id, files) in &ok {
        toggle_mod(&ctx, *id, files, true).unwrap();
    }

    // Uninstall the first mod and undo it.
    let (id, files) = &ok[0];
    let batch = uninstall_files(&ctx, *id, files).unwrap();
    assert!(!game.addons_dir.join(&files[0]).exists());
    journal.rollback_batch(&batch).unwrap();
    assert!(game.addons_dir.join(&files[0]).is_file());

    // Roll everything back, newest first: the addons folder ends up empty.
    let mut batches: Vec<String> = Vec::new();
    for e in journal.entries().unwrap() {
        if !e.rolled_back && !batches.contains(&e.batch) {
            batches.push(e.batch);
        }
    }
    for b in batches.iter().rev() {
        journal.rollback_batch(b).unwrap();
    }
    assert_eq!(scan_addons(&game.addons_dir).unwrap().len(), 0);
    assert_eq!(std::fs::read(&game.gameinfo).unwrap(), original);
}
