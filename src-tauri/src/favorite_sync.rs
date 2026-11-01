//! When favorites sync with the account: at app start, after a sign-in, on window focus, and
//! after a local change (debounced). A failed sync retries later. The sync logic is in
//! `favorite_sync_service`.

use crate::account::{emit_changed, Account};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{App, AppHandle, Emitter, Manager};
use youskill_core::services::favorite_api_service::FavoriteApi;
use youskill_core::services::favorite_sync_service::{
  FavoriteSync, RejectReason, SyncError, SyncReport,
};
use youskill_core::services::favorites_service;

/// Sent after each sync, so the favorite list reloads.
pub const SYNCED_EVENT: &str = "favorites://synced";

pub const NOW: Duration = Duration::ZERO;
pub const AFTER_FOCUS: Duration = Duration::from_secs(1);
pub const AFTER_CHANGE: Duration = Duration::from_secs(2);
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long sign-out waits for the push of pending changes.
const SIGN_OUT_PUSH_LIMIT: Duration = Duration::from_secs(5);

pub struct Sync {
  api: FavoriteApi,
  sync: FavoriteSync,
  /// The latest request; an earlier request that still waits does not run.
  requested: AtomicU64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritesSynced {
  /// Adds reverted because the account has the maximum number of favorites.
  pub limit_reached: Vec<String>,
  /// Changes the server rejected for another reason.
  pub rejected: Vec<String>,
}

impl From<SyncReport> for FavoritesSynced {
  fn from(report: SyncReport) -> Self {
    let mut synced = Self {
      limit_reached: Vec::new(),
      rejected: Vec::new(),
    };
    for (skill_id, reason) in report.rejected {
      match reason {
        RejectReason::LimitReached => synced.limit_reached.push(skill_id),
        RejectReason::Other(_) => synced.rejected.push(skill_id),
      }
    }
    synced
  }
}

/// Creates the sync state and runs the first sync.
pub fn setup(app: &App) -> Result<(), String> {
  app.manage(Sync {
    api: FavoriteApi::production()?,
    sync: FavoriteSync::new(favorites_service::store_path()?),
    requested: AtomicU64::new(0),
  });
  request(app.handle(), NOW);
  Ok(())
}

/// Runs a sync after `delay`, unless a later request replaces this one.
pub fn request(app: &AppHandle, delay: Duration) {
  // Before `setup`, the first sync of `setup` covers the request.
  let Some(state) = app.try_state::<Sync>() else {
    return;
  };
  let id = state.requested.fetch_add(1, Ordering::SeqCst) + 1;
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    if !delay.is_zero() {
      tokio::time::sleep(delay).await;
    }
    if app.state::<Sync>().requested.load(Ordering::SeqCst) == id {
      run(&app).await;
    }
  });
}

async fn run(app: &AppHandle) {
  let account = app.state::<Account>();
  let state = app.state::<Sync>();
  match state.sync.run(&account, &state.api.server(&account)).await {
    // Signed out: nothing to sync.
    Ok(None) => {},
    Ok(Some(report)) => {
      if let Err(error) = app.emit(SYNCED_EVENT, FavoritesSynced::from(report)) {
        tracing::warn!("Failed to send the favorite sync result: {error}");
      }
    },
    // The session is gone; local favorites stay for the next sign-in.
    Err(SyncError::Unauthorized) => emit_changed(app, None).await,
    Err(SyncError::Failed(error)) => {
      tracing::warn!("Favorite sync failed: {error}");
      request(app, RETRY_AFTER);
    },
  }
}

/// Pushes the pending changes (best effort), then signs out and clears the favorites.
pub async fn sign_out(app: &AppHandle) -> Result<(), String> {
  let account = app.state::<Account>();
  let state = app.state::<Sync>();
  state
    .sync
    .sign_out(&account, &state.api.server(&account), SIGN_OUT_PUSH_LIMIT)
    .await
}
