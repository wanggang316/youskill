//! Account sign-in and sign-out. The tokens stay in the backend; the frontend gets only the
//! identity.

use crate::account::{emit_changed, Account, AccountChanged, LastError};
use tauri::{AppHandle, State};
use youskill_core::services::account_service::{AccountError, AccountStatus, Provider};

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

#[tauri::command]
pub async fn account_sign_out(
  app: AppHandle,
  account: State<'_, Account>,
) -> Result<AccountStatus, String> {
  account.sign_out().await?;
  emit_changed(&app, None).await;
  Ok(account.status().await)
}

/// Deletes the account on the server and the account data of this device. The result has the
/// status after the attempt and the failure to show, if any.
#[tauri::command]
pub async fn account_delete(account: State<'_, Account>) -> Result<AccountChanged, String> {
  let error = match account.delete_account().await {
    Ok(()) => None,
    Err(AccountError::SignedOut) => {
      Some("The session ended. Sign in again to delete the account.".to_string())
    },
    Err(AccountError::Failed(message)) => Some(format!("Failed to delete the account: {message}")),
  };
  Ok(AccountChanged {
    status: account.status().await,
    error,
  })
}
