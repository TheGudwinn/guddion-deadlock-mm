use deadlock_mm_lib::steam::locate::*;
use std::fs;
use std::path::Path;

fn make_game(lib: &Path) {
    let citadel = lib.join("steamapps/common/Deadlock/game/citadel");
    fs::create_dir_all(&citadel).unwrap();
    fs::write(citadel.join("gameinfo.gi"), "x").unwrap();
    fs::write(
        lib.join("steamapps/appmanifest_1422450.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"1422450\"\n\t\"installdir\"\t\"Deadlock\"\n\t\"buildid\"\t\"123456\"\n}",
    )
    .unwrap();
}

#[test]
fn finds_game_in_secondary_library() {
    let d = tempfile::tempdir().unwrap();
    let steam = d.path().join("Steam");
    let lib2 = d.path().join("SteamLibrary");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    let esc = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
    let vdf = format!(
        "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\"{}\"\n\t}}\n}}",
        esc(&steam),
        esc(&lib2)
    );
    fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
    make_game(&lib2);

    assert_eq!(library_paths(&steam).len(), 2);
    let g = find_deadlock(&[steam]).expect("game found");
    assert_eq!(g.build_id.as_deref(), Some("123456"));
    assert!(g.gameinfo.ends_with("gameinfo.gi"));
    assert!(g.addons_dir.ends_with("addons"));
}

#[test]
fn not_installed_returns_none() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("steamapps")).unwrap();
    assert!(find_deadlock(&[d.path().to_path_buf()]).is_none());
}

#[test]
fn manual_path_accepts_root_game_or_citadel_and_rejects_others() {
    let d = tempfile::tempdir().unwrap();
    make_game(d.path());
    let root = d.path().join("steamapps/common/Deadlock");
    let a = validate_game_dir(&root).unwrap();
    assert_eq!(validate_game_dir(&root.join("game")).unwrap(), a);
    assert_eq!(validate_game_dir(&root.join("game/citadel")).unwrap(), a);
    assert!(matches!(validate_game_dir(d.path()), Err(GameDirError::NotDeadlock(_))));
    assert!(matches!(validate_game_dir(&d.path().join("nope")), Err(GameDirError::Missing(_))));
}

/// Read-only check against the real machine; does nothing when Deadlock is not installed.
#[test]
fn real_machine_detection_is_read_only() {
    if let Some(g) = auto_detect() {
        assert!(g.gameinfo.is_file());
        println!("detected: {:?} build {:?}", g.root, g.build_id);
    }
}

#[test]
fn manual_game_path_wins_and_a_bad_one_is_an_error() {
    use deadlock_mm_lib::steam::locate::resolve_game;
    let lib = tempfile::tempdir().unwrap();
    let root = lib.path().join("steamapps").join("common").join("Deadlock");
    std::fs::create_dir_all(root.join("game").join("citadel")).unwrap();
    std::fs::write(root.join("game").join("citadel").join("gameinfo.gi"), "x").unwrap();
    std::fs::write(
        lib.path().join("steamapps").join("appmanifest_1422450.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"1422450\"\n\t\"buildid\"\t\"777\"\n}\n",
    )
    .unwrap();

    let g = resolve_game(Some(root.to_str().unwrap())).unwrap().unwrap();
    assert!(g.addons_dir.ends_with("addons"));
    assert_eq!(g.build_id.as_deref(), Some("777"));

    let empty = lib.path().join("nothing");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(resolve_game(Some(empty.to_str().unwrap())).is_err());
    assert!(resolve_game(Some(lib.path().join("missing").to_str().unwrap())).is_err());
}
