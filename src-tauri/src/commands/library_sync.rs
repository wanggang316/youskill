//! Back up the library to GitHub and bring it to other computers. Thin adapters over
//! `library_sync_service`; network calls run on the blocking pool.

use crate::config::load_config;
use crate::models::InstallMode;
use crate::services::agent_apps_service::refresh_local_agent_apps;
use crate::services::env::Env;
use crate::services::library_sync_service::{
  self, ApplyPendingOutcome, BackupOutcome, PendingItem, PullOutcome, RemoteCheck, SyncStatus,
};

async fn blocking<T: Send + 'static>(
  f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
  tokio::task::spawn_blocking(f)
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn library_sync_status() -> Result<SyncStatus, String> {
  blocking(|| library_sync_service::status(&Env::current()?)).await
}

/// `repo` is `name` or `owner/name`; empty uses the default repository name.
#[tauri::command]
pub async fn library_sync_connect(repo: Option<String>) -> Result<SyncStatus, String> {
  blocking(move || library_sync_service::connect(&Env::current()?, &repo.unwrap_or_default())).await
}

#[tauri::command]
pub async fn library_sync_disconnect() -> Result<(), String> {
  blocking(|| library_sync_service::disconnect(&Env::current()?)).await
}

#[tauri::command]
pub async fn library_sync_backup() -> Result<BackupOutcome, String> {
  blocking(|| library_sync_service::backup(&Env::current()?)).await
}

#[tauri::command]
pub async fn library_sync_check_remote() -> Result<RemoteCheck, String> {
  blocking(|| library_sync_service::check_remote(&Env::current()?)).await
}

#[tauri::command]
pub async fn library_sync_pull() -> Result<PullOutcome, String> {
  blocking(|| {
    let outcome = library_sync_service::pull(&Env::current()?)?;
    if outcome.agents_changed {
      refresh_local_agent_apps();
    }
    Ok(outcome)
  })
  .await
}

#[tauri::command]
pub async fn library_sync_pending() -> Result<Vec<PendingItem>, String> {
  blocking(|| library_sync_service::pending(&Env::current()?)).await
}

/// Installs use the default install mode from Settings > General.
#[tauri::command]
pub async fn library_sync_apply_pending(
  items: Vec<PendingItem>,
) -> Result<ApplyPendingOutcome, String> {
  blocking(move || {
    let mode = InstallMode::from_setting(&load_config()?.sync_mode);
    library_sync_service::apply_pending(&Env::current()?, &items, mode)
  })
  .await
}
