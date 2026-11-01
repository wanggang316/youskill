//! Account sign-in wiring: the OS keychain as the secret store, the `youskill://` deep link,
//! and the `account://changed` event. The session logic is in `account_service`.

use serde::Serialize;
use tauri::{App, AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use youskill_core::services::account_service::{
  system_clock, AccountError, AccountManager, AccountStatus, CallbackOutcome, HttpAccountServer,
  SecretStore,
};
use youskill_core::services::favorites_service;

pub type Account = AccountManager<HttpAccountServer, Keychain>;

/// Sent when the account state changes outside a command: a sign-in callback or a session
/// the server revoked.
pub const CHANGED_EVENT: &str = "account://changed";

const KEYCHAIN_SERVICE: &str = "com.gumpw.youskill.account";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountChanged {
  pub status: AccountStatus,
  /// A sign-in failure to show. A cancelled sign-in has none.
  pub error: Option<String>,
}

/// The last sign-in failure, kept until the frontend reads it: a link that starts the app is
/// handled before the page listens for `CHANGED_EVENT`.
#[derive(Default)]
pub struct LastError(std::sync::Mutex<Option<String>>);

impl LastError {
  pub fn take(&self) -> Option<String> {
    self.0.lock().ok().and_then(|mut error| error.take())
  }

  fn set(&self, error: Option<String>) {
    if let Ok(mut current) = self.0.lock() {
      *current = error;
    }
  }
}

/// Tokens and the pending sign-in, one keychain item per key.
pub struct Keychain;

impl Keychain {
  fn entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, key).map_err(|error| error.to_string())
  }
}

impl SecretStore for Keychain {
  fn load(&self, key: &str) -> Result<Option<String>, String> {
    match Self::entry(key)?.get_password() {
      Ok(value) => Ok(Some(value)),
      Err(keyring::Error::NoEntry) => Ok(None),
      Err(error) => Err(error.to_string()),
    }
  }

  fn save(&self, key: &str, value: &str) -> Result<(), String> {
    Self::entry(key)?
      .set_password(value)
      .map_err(|error| error.to_string())
  }

  fn delete(&self, key: &str) -> Result<(), String> {
    match Self::entry(key)?.delete_credential() {
      Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
      Err(error) => Err(error.to_string()),
    }
  }
}

/// Creates the account state and handles sign-in callbacks: links that arrive while the app
/// runs, links that a second instance forwards, and the link that started the app.
pub fn setup(app: &App) -> Result<(), String> {
  let account = AccountManager::new(
    HttpAccountServer::production()?,
    Keychain,
    favorites_service::store_path()?,
    system_clock(),
  );
  app.manage(account);
  app.manage(LastError::default());

  // Windows installers and macOS bundles register the scheme; Linux and Windows dev builds
  // must register it at run time.
  #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
  if let Err(error) = app.deep_link().register_all() {
    tracing::warn!("Failed to register the deep link scheme: {error}");
  }

  let handle = app.handle().clone();
  app.deep_link().on_open_url(move |event| {
    for url in event.urls() {
      handle_link(&handle, url.to_string());
    }
  });
  if let Ok(Some(urls)) = app.deep_link().get_current() {
    for url in urls {
      handle_link(app.handle(), url.to_string());
    }
  }

  // An expired session refreshes now, so a revoked one shows as signed out.
  let handle = app.handle().clone();
  tauri::async_runtime::spawn(async move {
    let account = handle.state::<Account>();
    let was_signed_in = account.status().await.identity.is_some();
    if was_signed_in && account.access_token().await == Err(AccountError::SignedOut) {
      emit_changed(&handle, None).await;
    }
  });
  Ok(())
}

fn handle_link(app: &AppHandle, link: String) {
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    let outcome = app.state::<Account>().complete_sign_in(&link).await;
    let error = match outcome {
      Ok(CallbackOutcome::Ignored) => return,
      Ok(CallbackOutcome::SignedIn(_) | CallbackOutcome::Cancelled) => None,
      Err(error) => Some(error),
    };
    app.state::<LastError>().set(error.clone());
    if let Some(window) = app.get_webview_window("main") {
      let _ = window.show();
      let _ = window.unminimize();
      let _ = window.set_focus();
    }
    emit_changed(&app, error).await;
  });
}

pub async fn emit_changed(app: &AppHandle, error: Option<String>) {
  let status = app.state::<Account>().status().await;
  if let Err(error) = app.emit(CHANGED_EVENT, AccountChanged { status, error }) {
    tracing::warn!("Failed to send the account change: {error}");
  }
}
