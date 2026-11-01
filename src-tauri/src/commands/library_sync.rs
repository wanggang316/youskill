//! Back up the library to GitHub and bring it to other computers. Thin adapters over
//! `library_sync_service`; network calls run on the blocking pool.

use crate::config::load_config;
use crate::models::InstallMode;
use crate::services::agent_apps_service::refresh_local_agent_apps;
use crate::services::env::Env;
use crate::services::library_sync_service::{
  self, ApplyPendingOutcome, BackupOutcome, DeviceInfo, DeviceItemChoice, ItemScope, PendingItem,
  RemoteCommit, RestoreDecisions, RestoreMode, RestoreOptions, RestoreOutcome, RestorePlan,
  SyncStatus,
};
use std::collections::BTreeMap;

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

/// Migrate a version 1 repository to item scopes and device files, after the user
/// confirmed it. Other computers must update the app before they sync again.
#[tauri::command]
pub async fn library_sync_migrate() -> Result<(), String> {
  blocking(|| library_sync_service::migrate(&Env::current()?)).await
}

/// Compare this computer's library with the branch head, or with `target`, an earlier
/// commit of the branch.
#[tauri::command]
pub async fn library_sync_plan_restore(
  mode: RestoreMode,
  target: Option<String>,
) -> Result<RestorePlan, String> {
  blocking(move || library_sync_service::plan_restore(&Env::current()?, mode, target.as_deref()))
    .await
}

/// Installs use the default install mode from Settings > General.
#[tauri::command]
pub async fn library_sync_apply_restore(
  plan: RestorePlan,
  decisions: RestoreDecisions,
  options: RestoreOptions,
) -> Result<RestoreOutcome, String> {
  blocking(move || {
    let mode = InstallMode::from_setting(&load_config()?.sync_mode);
    let outcome =
      library_sync_service::apply_restore(&Env::current()?, &plan, &decisions, &options, mode)?;
    if outcome.agents_changed {
      refresh_local_agent_apps();
    }
    Ok(outcome)
  })
  .await
}

#[tauri::command]
pub async fn library_sync_undo_restore() -> Result<(), String> {
  blocking(|| {
    library_sync_service::undo_restore(&Env::current()?)?;
    refresh_local_agent_apps();
    Ok(())
  })
  .await
}

#[tauri::command]
pub async fn library_sync_history() -> Result<Vec<RemoteCommit>, String> {
  blocking(|| library_sync_service::history(&Env::current()?)).await
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

/// The scope of a library item (`skill:<id>`, `instruction:<id>`, `agent:<id>`); `null`
/// when the library is not connected.
#[tauri::command]
pub async fn library_sync_item_scope(key: String) -> Result<Option<ItemScope>, String> {
  blocking(move || library_sync_service::item_scope(&Env::current()?, &key)).await
}

#[tauri::command]
pub async fn library_sync_set_item_scope(key: String, scope: ItemScope) -> Result<(), String> {
  blocking(move || library_sync_service::set_item_scope(&Env::current()?, &key, scope)).await
}

/// Copy a device-only item of another device as a device-only item of this device. Returns
/// the key of the copy.
#[tauri::command]
pub async fn library_sync_copy_device_item(key: String) -> Result<String, String> {
  blocking(move || {
    let copied = library_sync_service::copy_device_item(&Env::current()?, &key)?;
    if copied.starts_with("agent:") {
      refresh_local_agent_apps();
    }
    Ok(copied)
  })
  .await
}

#[tauri::command]
pub async fn library_sync_devices() -> Result<Vec<DeviceInfo>, String> {
  blocking(|| library_sync_service::devices(&Env::current()?)).await
}

#[tauri::command]
pub async fn library_sync_rename_device(name: String) -> Result<SyncStatus, String> {
  blocking(move || library_sync_service::rename_device(&Env::current()?, &name)).await
}

/// First connect only: this computer becomes `device`.
#[tauri::command]
pub async fn library_sync_take_over_device(device: String) -> Result<SyncStatus, String> {
  blocking(move || library_sync_service::take_over_device(&Env::current()?, &device)).await
}

/// First connect only: start from the installs of `from`, or of the device with the latest
/// backup.
#[tauri::command]
pub async fn library_sync_copy_device_installs(from: Option<String>) -> Result<SyncStatus, String> {
  blocking(move || library_sync_service::copy_device_installs(&Env::current()?, from.as_deref()))
    .await
}

#[tauri::command]
pub async fn library_sync_remove_device(
  device: String,
  choices: BTreeMap<String, DeviceItemChoice>,
) -> Result<(), String> {
  blocking(move || library_sync_service::remove_device(&Env::current()?, &device, &choices)).await
}
