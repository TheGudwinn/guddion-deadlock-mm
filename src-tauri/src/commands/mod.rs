//! Tauri commands. Thin wrappers: all logic lives in the library modules.
mod game;
mod library;
mod mod_updates;
mod profiles;
mod queue;
mod settings;
mod updates;

pub use game::*;
pub use library::*;
pub use mod_updates::*;
pub use profiles::*;
pub use queue::*;
pub use settings::*;
pub use updates::*;

use crate::db::Db;
use crate::net::NetService;
use crate::sources::cache::CachedSource;
use crate::sources::gamebanana::GameBanana;
use crate::sources::{Category, ModDetails, ModSource, SearchPage, SearchQuery};
use crate::settings::AppSettings;
use crate::steam::locate::{resolve_game, validate_game_dir, GameInstall};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::{Mutex, OnceCell};

pub struct AppState {
    pub net: Arc<NetService>,
    pub gb: Arc<GameBanana>,
    pub app_dir: PathBuf,
    pub db: OnceCell<Db>,
    /// Serializes operations that change game files.
    pub busy: Mutex<()>,
    pub settings: std::sync::Mutex<AppSettings>,
    pub queue: Arc<crate::queue::Queue>,
}

impl AppState {
    pub fn new(net: Arc<NetService>, gb: Arc<GameBanana>, app_dir: PathBuf, settings: AppSettings) -> Self {
        Self { net, gb, app_dir, db: OnceCell::new(), busy: Mutex::new(()), settings: std::sync::Mutex::new(settings), queue: Arc::new(crate::queue::Queue::new()) }
    }

    pub fn settings_path(&self) -> PathBuf {
        self.app_dir.join("settings.json")
    }

    /// The game folder: the manual path from settings, else Steam auto-detection.
    /// A manual path that no longer points at Deadlock is an error, not a silent fallback.
    pub fn game(&self) -> Result<Option<GameInstall>, String> {
        let manual = self.settings.lock().unwrap().game_path.clone();
        resolve_game(manual.as_deref()).map_err(|e| e.to_string())
    }

    /// Starts the local Postgres on first use.
    pub async fn db(&self) -> Result<&Db, String> {
        self.db.get_or_try_init(|| Db::start(&self.app_dir)).await.map_err(|e| e.to_string())
    }

    /// The catalog source; wrapped with the offline cache once the database is up.
    pub fn source(&self) -> Arc<dyn ModSource> {
        match self.db.get() {
            Some(db) => Arc::new(CachedSource::new(self.gb.clone(), db.pool.clone())),
            None => self.gb.clone(),
        }
    }
}

pub(crate) fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn detect_game(state: State<'_, AppState>) -> Result<Option<GameInstall>, String> {
    state.game()
}

#[tauri::command]
pub fn check_game_path(path: String) -> Result<GameInstall, String> {
    validate_game_dir(&PathBuf::from(path)).map_err(err)
}

#[tauri::command]
pub async fn search_mods(state: State<'_, AppState>, query: SearchQuery) -> Result<SearchPage, String> {
    state.source().search(&query).await.map_err(err)
}

#[tauri::command]
pub async fn mod_details(state: State<'_, AppState>, id: String) -> Result<ModDetails, String> {
    state.source().get_mod_details(&id).await.map_err(err)
}

#[tauri::command]
pub async fn list_categories(state: State<'_, AppState>, parent: Option<i64>) -> Result<Vec<Category>, String> {
    state.source().categories(parent).await.map_err(err)
}
