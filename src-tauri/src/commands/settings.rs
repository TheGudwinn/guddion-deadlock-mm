//! Settings and network diagnostics commands.
use super::{err, AppState};
use crate::net::config::ProxyMode;
use crate::net::diagnose::{diagnose_all, HostReport};
use crate::settings::{self, SettingsInput, SettingsView, SOURCE_GAMEBANANA};
use crate::steam::locate::{resolve_game, GameInstall};
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsView {
    state.settings.lock().unwrap().view()
}

/// Validates, applies to the live network client, then persists. Nothing changes if any step fails.
#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, input: SettingsInput) -> Result<SettingsView, String> {
    let next = state.settings.lock().unwrap().with_input(input).map_err(err)?;
    // A manual game folder must be valid at save time (the error names the path).
    resolve_game(next.game_path.as_deref()).map_err(err)?;
    state.net.update_config(next.net.clone()).map_err(err)?;
    settings::save(&state.settings_path(), &next).map_err(err)?;
    let view = next.view();
    *state.settings.lock().unwrap() = next;
    Ok(view)
}

/// Resolves the game folder for a path typed or picked in the UI without saving anything.
#[tauri::command]
pub fn check_game_folder(path: String) -> Result<GameInstall, String> {
    resolve_game(Some(&path)).map_err(err)?.ok_or_else(|| "game_not_found".to_string())
}

/// Checks every host the app talks to, through the current proxy settings.
#[tauri::command]
pub async fn run_diagnostics(state: State<'_, AppState>) -> Result<Vec<HostReport>, String> {
    let cfg = state.net.config();
    let mut urls: Vec<String> = cfg.mirrors.get(SOURCE_GAMEBANANA).cloned().unwrap_or_default();
    // Update manifest hosts (GitHub is often the first thing to break) are checked with the same steps.
    urls.extend(state.settings.lock().unwrap().update_endpoints.clone());
    let via_proxy = cfg.proxy.mode == ProxyMode::Custom;
    Ok(diagnose_all(&state.net.client(), &urls, via_proxy).await)
}
