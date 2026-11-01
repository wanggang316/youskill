//! Account sign-in and sign-out. The tokens stay in the backend; the frontend gets only the
//! identity.

use crate::account::{emit_changed, Account, AccountChanged, LastError};
use tauri::{AppHandle, State};
use youskill_core::services::account_service::{AccountStatus, Provider};

/// The status and the last sign-in failure that is not shown yet.
#[tauri::command]
pub async fn account_status(
  account: State<'_, Account>,
  last_error: State<'_, LastError>,
) -> Result<AccountChanged, String> {
  Ok(AccountChanged {
    status: account.status().await,
    error: last_error.take(),
  })
}

/// Starts a sign-in and returns the URL for the system browser. The result arrives as an
/// `account://changed` event after the `youskill://auth/callback` link.
#[tauri::command]
pub fn account_sign_in(
  account: State<'_, Account>,
  last_error: State<'_, LastError>,
  provider: Provider,
) -> Result<String, String> {
  // A failure of an earlier sign-in must not show for this one.
  last_error.take();
  account.begin_sign_in(provider)
}

/// Pushes pending favorite changes (best effort), then signs out and clears the favorites.
#[tauri::command]
pub async fn account_sign_out(
  app: AppHandle,
  account: State<'_, Account>,
) -> Result<AccountStatus, String> {
  crate::favorite_sync::sign_out(&app).await?;
  emit_changed(&app, None).await;
  Ok(account.status().await)
}
