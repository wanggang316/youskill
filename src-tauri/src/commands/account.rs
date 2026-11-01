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
pub fn account_sign_in(account: State<'_, Account>, provider: Provider) -> Result<String, String> {
  account.begin_sign_in(provider)
}

#[tauri::command]
pub async fn account_sign_out(
  app: AppHandle,
  account: State<'_, Account>,
) -> Result<AccountStatus, String> {
  account.sign_out().await?;
  emit_changed(&app, None).await;
  Ok(account.status().await)
}
