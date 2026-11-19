use super::vdf;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const DEADLOCK_APP_ID: &str = "1422450";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GameInstall {
    pub root: PathBuf,
    pub citadel_dir: PathBuf,
    pub addons_dir: PathBuf,
    pub gameinfo: PathBuf,
    /// Steam build id from the app manifest, when the game was found via Steam.
    pub build_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GameDirError {
    #[error("path does not exist: {0}")]
    Missing(PathBuf),
    #[error("not a Deadlock folder (game/citadel/gameinfo.gi not found): {0}")]
    NotDeadlock(PathBuf),
}

/// Accepts the Deadlock root, its `game` folder or `game/citadel` and returns the resolved layout.
pub fn validate_game_dir(input: &Path) -> Result<GameInstall, GameDirError> {
    if !input.exists() {
        return Err(GameDirError::Missing(input.to_path_buf()));
    }
    let candidates = [input.to_path_buf(), input.join(".."), input.join("..").join("..")];
    for c in candidates {
        let Ok(root) = c.canonicalize().map(strip_verbatim) else { continue };
        let citadel = root.join("game").join("citadel");
        let gameinfo = citadel.join("gameinfo.gi");
        if gameinfo.is_file() {
            return Ok(GameInstall {
                addons_dir: citadel.join("addons"),
                citadel_dir: citadel,
                gameinfo,
                root,
                build_id: None,
            });
        }
    }
    Err(GameDirError::NotDeadlock(input.to_path_buf()))
}

/// Windows canonicalize() returns `\?\C:\...`; drop the prefix for display and comparisons.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// Steam library folders: the root itself plus every `path` in libraryfolders.vdf.
pub fn library_paths(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let file = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(&file) {
        if let Ok(v) = vdf::parse(&text) {
            if let Some(root) = v.get("libraryfolders") {
                for (_, entry) in root.entries() {
                    // Old format: "1" "D:\\Lib" (plain string); new: object with "path".
                    let p = entry.as_str().or_else(|| entry.get("path").and_then(|p| p.as_str()));
                    if let Some(p) = p {
                        let p = PathBuf::from(p);
                        if !libs.iter().any(|l| same_path(l, &p)) {
                            libs.push(p);
                        }
                    }
                }
            }
        }
    }
    libs
}

/// Look for the Deadlock app manifest in every library of every Steam root.
pub fn find_deadlock(steam_roots: &[PathBuf]) -> Option<GameInstall> {
    for root in steam_roots {
        for lib in library_paths(root) {
            let manifest = lib.join("steamapps").join(format!("appmanifest_{DEADLOCK_APP_ID}.acf"));
            let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
            let Ok(v) = vdf::parse(&text) else { continue };
            let Some(state) = v.get("AppState") else { continue };
            let installdir = state.get("installdir").and_then(|s| s.as_str()).unwrap_or("Deadlock");
            let dir = lib.join("steamapps").join("common").join(installdir);
            if let Ok(mut g) = validate_game_dir(&dir) {
                g.build_id = state.get("buildid").and_then(|s| s.as_str()).map(str::to_string);
                return Some(g);
            }
        }
    }
    None
}

/// Steam install folders from the registry and common default locations.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let reads = [
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"),
            (HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
        ];
        for (hive, key, name) in reads {
            if let Ok(k) = RegKey::predef(hive).open_subkey(key) {
                if let Ok(p) = k.get_value::<String, _>(name) {
                    out.push(PathBuf::from(p));
                }
            }
        }
        out.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME") {
        let h = PathBuf::from(home);
        out.push(h.join(".steam/steam"));
        out.push(h.join(".local/share/Steam"));
    }
    let mut uniq: Vec<PathBuf> = Vec::new();
    for p in out {
        if p.exists() && !uniq.iter().any(|u| same_path(u, &p)) {
            uniq.push(p);
        }
    }
    uniq
}

pub fn auto_detect() -> Option<GameInstall> {
    find_deadlock(&steam_roots())
}

/// A manually chosen folder wins over auto-detection. A bad manual path is an error,
/// never a silent fallback: installing into a different copy than the user picked would be a surprise.
pub fn resolve_game(manual: Option<&str>) -> Result<Option<GameInstall>, GameDirError> {
    match manual.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            let mut g = validate_game_dir(Path::new(p))?;
            g.build_id = build_id_near(&g.root);
            Ok(Some(g))
        }
        None => Ok(auto_detect()),
    }
}

/// `<library>/steamapps/common/<dir>` -> read `<library>/steamapps/appmanifest_<id>.acf` if it exists.
fn build_id_near(root: &Path) -> Option<String> {
    let steamapps = root.parent()?.parent()?;
    let text = std::fs::read_to_string(steamapps.join(format!("appmanifest_{DEADLOCK_APP_ID}.acf"))).ok()?;
    let v = vdf::parse(&text).ok()?;
    v.get("AppState")?.get("buildid")?.as_str().map(str::to_string)
}
