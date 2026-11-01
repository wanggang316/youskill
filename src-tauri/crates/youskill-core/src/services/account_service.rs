//! Account sign-in session of this device, by the youskill-web account API
//! (`youskill-web/docs/account-api.md`).
//!
//! Sign-in runs in the system browser with PKCE and returns through the
//! `youskill://auth/callback` deep link. The tokens and the pending sign-in are kept only in
//! the secret store (the OS keychain in the app). `AccountServer` is the boundary to the
//! account API; `SecretStore` is the boundary to the keychain.

use crate::services::favorites_service;
use base64::Engine;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

pub const REDIRECT_URI: &str = "youskill://auth/callback";
/// The access token refreshes when it expires within this time.
const REFRESH_MARGIN_SECS: i64 = 60;
/// A pending sign-in older than this is not completed. The server code lives 5 minutes.
const PENDING_TTL_SECS: i64 = 10 * 60;
const SESSION_KEY: &str = "session";
const PENDING_KEY: &str = "pending-sign-in";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
  Github,
  Apple,
}

impl Provider {
  pub fn as_str(self) -> &'static str {
    match self {
      Provider::Github => "github",
      Provider::Apple => "apple",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
  pub provider: String,
  #[serde(default)]
  pub display_name: Option<String>,
  #[serde(default)]
  pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
  pub id: i64,
  #[serde(default)]
  pub display_name: Option<String>,
  #[serde(default)]
  pub email: Option<String>,
  #[serde(default)]
  pub identities: Vec<Identity>,
}

/// Response of `POST /account/token` and `POST /account/refresh`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionResponse {
  pub access_token: String,
  pub expires_in: i64,
  pub refresh_token: String,
  pub account: Account,
}

/// The session as it is kept in the secret store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredSession {
  access_token: String,
  refresh_token: String,
  /// Unix seconds.
  expires_at: i64,
  provider: Provider,
  account: Account,
}

/// A sign-in that waits for its callback. Kept in the secret store, so a callback that
/// starts the app can still complete it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PendingSignIn {
  provider: Provider,
  state: String,
  code_verifier: String,
  /// Unix seconds.
  created_at: i64,
}

/// What the frontend shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
  /// `None` when signed out.
  pub identity: Option<AccountIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountIdentity {
  pub account_id: String,
  pub provider: Provider,
  pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
  /// `401`: the token is missing, expired or revoked.
  Unauthorized,
  /// Another `4xx`, such as a used or expired code.
  Rejected(String),
  /// Network failure, rate limit or server failure.
  Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
  /// No session, or the server revoked it. The app shows the signed-out state.
  SignedOut,
  /// The session stays; try again later.
  Failed(String),
}

/// Result of a deep link given to `complete_sign_in`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackOutcome {
  SignedIn(AccountIdentity),
  /// The person cancelled in the browser.
  Cancelled,
  /// Not a sign-in callback, or no sign-in waits for it (for example a second delivery).
  Ignored,
}

pub trait AccountServer {
  /// Base of the start URL, such as `https://api.youskill.cc`.
  fn base_url(&self) -> &str;

  fn exchange(
    &self,
    code: &str,
    code_verifier: &str,
  ) -> impl Future<Output = Result<SessionResponse, ServerError>> + Send;

  fn refresh(
    &self,
    refresh_token: &str,
  ) -> impl Future<Output = Result<SessionResponse, ServerError>> + Send;

  fn sign_out(&self, access_token: &str) -> impl Future<Output = Result<(), ServerError>> + Send;

  /// `DELETE /account`: deletes the account and all its data on the server.
  fn delete_account(
    &self,
    access_token: &str,
  ) -> impl Future<Output = Result<(), ServerError>> + Send;
}

pub trait SecretStore {
  fn load(&self, key: &str) -> Result<Option<String>, String>;
  fn save(&self, key: &str, value: &str) -> Result<(), String>;
  /// Deleting a missing key is not an error.
  fn delete(&self, key: &str) -> Result<(), String>;
}

pub type Clock = Box<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
  Box::new(|| chrono::Utc::now().timestamp())
}

pub struct AccountManager<S, K> {
  server: S,
  secrets: K,
  favorites_path: PathBuf,
  now: Clock,
  /// Held across a refresh, so only one refresh runs at a time.
  session: tokio::sync::Mutex<Option<StoredSession>>,
  /// The same link can arrive twice at start-up; only one may use the code.
  completing: tokio::sync::Mutex<()>,
}

impl<S: AccountServer, K: SecretStore> AccountManager<S, K> {
  /// Loads the stored session. A session that cannot be read counts as signed out.
  pub fn new(server: S, secrets: K, favorites_path: PathBuf, now: Clock) -> Self {
    let session = secrets
      .load(SESSION_KEY)
      .and_then(|value| value.map(|value| parse(&value)).transpose())
      .unwrap_or_else(|error| {
        tracing::warn!("Ignoring the stored account session: {error}");
        None
      });
    Self {
      server,
      secrets,
      favorites_path,
      now,
      session: tokio::sync::Mutex::new(session),
      completing: tokio::sync::Mutex::new(()),
    }
  }

  pub async fn status(&self) -> AccountStatus {
    AccountStatus {
      identity: self.session.lock().await.as_ref().map(identity),
    }
  }

  /// Starts a sign-in and returns the URL to open in the system browser. A new sign-in
  /// replaces one that is still pending.
  pub fn begin_sign_in(&self, provider: Provider) -> Result<String, String> {
    let pending = PendingSignIn {
      provider,
      state: random_token(1),
      code_verifier: random_token(2),
      created_at: (self.now)(),
    };
    let url = start_url(
      self.server.base_url(),
      provider,
      &pending.state,
      &pending.code_verifier,
    )?;
    self.secrets.save(PENDING_KEY, &to_json(&pending)?)?;
    Ok(url)
  }

  /// Completes the pending sign-in with the callback deep link.
  pub async fn complete_sign_in(&self, link: &str) -> Result<CallbackOutcome, String> {
    let Some(callback) = parse_callback(link) else {
      return Ok(CallbackOutcome::Ignored);
    };
    let _completing = self.completing.lock().await;
    let pending: Option<PendingSignIn> = self
      .secrets
      .load(PENDING_KEY)?
      .map(|value| parse(&value))
      .transpose()?;
    let Some(pending) = pending else {
      return Ok(CallbackOutcome::Ignored);
    };
    let cancelled = callback.error.as_deref() == Some("access_denied");
    if callback.state.as_deref() != Some(pending.state.as_str()) {
      // Keep the pending sign-in: the real callback can still arrive. A cancel of a sign-in
      // that a newer one replaced is no error.
      if cancelled {
        return Ok(CallbackOutcome::Ignored);
      }
      return Err("The sign-in response does not match this sign-in. Try again.".to_string());
    }
    // The code is single-use, so the pending sign-in is too.
    self.secrets.delete(PENDING_KEY)?;
    if cancelled {
      return Ok(CallbackOutcome::Cancelled);
    }
    if (self.now)() - pending.created_at > PENDING_TTL_SECS {
      return Err("The sign-in took too long. Try again.".to_string());
    }
    match (callback.code, callback.error) {
      (_, Some(error)) => Err(format!("Sign-in failed ({error}). Try again.")),
      (Some(code), None) => {
        let response = self
          .server
          .exchange(&code, &pending.code_verifier)
          .await
          .map_err(|error| match error {
            ServerError::Unauthorized | ServerError::Rejected(_) => {
              "The sign-in is not valid any more. Try again.".to_string()
            },
            ServerError::Failed(message) => format!("Sign-in failed: {message}"),
          })?;
        let mut session = self.session.lock().await;
        let stored = self.stored(response, pending.provider);
        let signed_in = identity(&stored);
        self.keep(&mut session, stored);
        Ok(CallbackOutcome::SignedIn(signed_in))
      },
      (None, None) => Err("The sign-in response has no code. Try again.".to_string()),
    }
  }

  /// A valid access token. It refreshes when it expires within the refresh margin.
  pub async fn access_token(&self) -> Result<String, AccountError> {
    let mut session = self.session.lock().await;
    self.access_token_locked(&mut session).await
  }

  async fn access_token_locked(
    &self,
    session: &mut Option<StoredSession>,
  ) -> Result<String, AccountError> {
    let current = session.as_ref().ok_or(AccountError::SignedOut)?;
    if (self.now)() + REFRESH_MARGIN_SECS < current.expires_at {
      return Ok(current.access_token.clone());
    }
    self.refresh_locked(session).await
  }

  /// A new access token after the server gave `401` for `rejected`. When another caller
  /// already refreshed, its token is returned without a second refresh.
  pub async fn refresh_rejected(&self, rejected: &str) -> Result<String, AccountError> {
    let mut session = self.session.lock().await;
    let current = session.as_ref().ok_or(AccountError::SignedOut)?;
    if current.access_token != rejected {
      return Ok(current.access_token.clone());
    }
    self.refresh_locked(&mut session).await
  }

  async fn refresh_locked(
    &self,
    session: &mut Option<StoredSession>,
  ) -> Result<String, AccountError> {
    let Some(current) = session.as_ref() else {
      return Err(AccountError::SignedOut);
    };
    match self.server.refresh(&current.refresh_token).await {
      Ok(response) => {
        let stored = self.stored(response, current.provider);
        let token = stored.access_token.clone();
        self.keep(session, stored);
        Ok(token)
      },
      Err(ServerError::Unauthorized) => {
        self.forget_revoked(session);
        Err(AccountError::SignedOut)
      },
      Err(ServerError::Rejected(message) | ServerError::Failed(message)) => {
        Err(AccountError::Failed(message))
      },
    }
  }

  /// Revokes the session on the server (best effort), deletes the tokens, and clears the
  /// favorites and the sync cursor of this device.
  pub async fn sign_out(&self) -> Result<(), String> {
    let token = {
      let mut session = self.session.lock().await;
      let token = self.revocable_token(&mut session).await;
      *session = None;
      token
    };
    let cleared = self
      .secrets
      .delete(SESSION_KEY)
      .and_then(|()| self.secrets.delete(PENDING_KEY))
      .and_then(|()| favorites_service::clear_store(&self.favorites_path));
    // The lock is released: a slow server does not block other callers.
    if let Some(token) = token {
      if let Err(error) = self.server.sign_out(&token).await {
        tracing::warn!("Failed to revoke the account session: {error:?}");
      }
    }
    cleared
  }

  /// The session the server does not accept any more. Local favorites stay for the next
  /// sign-in.
  fn forget_revoked(&self, session: &mut Option<StoredSession>) {
    *session = None;
    if let Err(error) = self.secrets.delete(SESSION_KEY) {
      tracing::warn!("Failed to delete the revoked account session: {error}");
    }
  }

  /// Deletes the account on the server, then deletes the tokens and clears the favorites and
  /// the sync cursor of this device. On a failure nothing local changes. A `401` refreshes
  /// once; a `401` that the refresh does not fix signs out and keeps the favorites, as for a
  /// revoked session.
  pub async fn delete_account(&self) -> Result<(), AccountError> {
    // The lock is held across the request: a refresh in this time could keep a session of
    // the deleted account.
    let mut session = self.session.lock().await;
    let mut token = self.access_token_locked(&mut session).await?;
    let mut refreshed = false;
    loop {
      match self.server.delete_account(&token).await {
        Ok(()) => break,
        Err(ServerError::Unauthorized) if !refreshed => {
          refreshed = true;
          token = self.refresh_locked(&mut session).await?;
        },
        Err(ServerError::Unauthorized) => {
          self.forget_revoked(&mut session);
          return Err(AccountError::SignedOut);
        },
        Err(ServerError::Rejected(message) | ServerError::Failed(message)) => {
          return Err(AccountError::Failed(message));
        },
      }
    }
    *session = None;
    self
      .secrets
      .delete(SESSION_KEY)
      .and_then(|()| self.secrets.delete(PENDING_KEY))
      .and_then(|()| favorites_service::clear_store(&self.favorites_path))
      .map_err(AccountError::Failed)
  }

  /// An access token the server accepts for sign-out. An expired one refreshes first, so the
  /// refresh token is revoked too. `None` when the server already revoked the session.
  async fn revocable_token(&self, session: &mut Option<StoredSession>) -> Option<String> {
    let current = session.as_ref()?;
    if (self.now)() + REFRESH_MARGIN_SECS < current.expires_at {
      return Some(current.access_token.clone());
    }
    let expired = current.access_token.clone();
    match self.refresh_locked(session).await {
      Ok(token) => Some(token),
      Err(AccountError::SignedOut) => None,
      // Try anyway: the token can still be valid within the margin.
      Err(AccountError::Failed(_)) => Some(expired),
    }
  }

  /// Keeps a new session in memory first: the server rotated the refresh token, so a failed
  /// save must not bring back the old one. It is then kept for the next start.
  fn keep(&self, session: &mut Option<StoredSession>, stored: StoredSession) {
    let saved = to_json(&stored).and_then(|json| self.secrets.save(SESSION_KEY, &json));
    if let Err(error) = saved {
      tracing::warn!("Failed to keep the account session in the keychain: {error}");
    }
    *session = Some(stored);
  }

  fn stored(&self, response: SessionResponse, provider: Provider) -> StoredSession {
    StoredSession {
      access_token: response.access_token,
      refresh_token: response.refresh_token,
      expires_at: (self.now)() + response.expires_in,
      provider,
      account: response.account,
    }
  }
}

fn identity(session: &StoredSession) -> AccountIdentity {
  let account = &session.account;
  let identity = account
    .identities
    .iter()
    .find(|identity| identity.provider == session.provider.as_str());
  let display_name = identity
    .and_then(|identity| identity.display_name.clone())
    .or(account.display_name.clone())
    .or(identity.and_then(|identity| identity.email.clone()))
    .or(account.email.clone())
    .unwrap_or_default();
  AccountIdentity {
    account_id: account.id.to_string(),
    provider: session.provider,
    display_name,
  }
}

/// `count` UUID v4 values as hex: 32 characters each, from the OS random source.
fn random_token(count: usize) -> String {
  (0..count)
    .map(|_| uuid::Uuid::new_v4().simple().to_string())
    .collect()
}

fn code_challenge(code_verifier: &str) -> String {
  base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier))
}

fn start_url(
  base_url: &str,
  provider: Provider,
  state: &str,
  code_verifier: &str,
) -> Result<String, String> {
  let url = Url::parse_with_params(
    &format!("{base_url}/api/account/sign-in/{}", provider.as_str()),
    [
      ("redirect_uri", REDIRECT_URI),
      ("code_challenge", &code_challenge(code_verifier)),
      ("code_challenge_method", "S256"),
      ("state", state),
    ],
  )
  .map_err(|error| error.to_string())?;
  Ok(url.into())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Callback {
  code: Option<String>,
  state: Option<String>,
  error: Option<String>,
}

fn parse_callback(link: &str) -> Option<Callback> {
  let url = Url::parse(link).ok()?;
  let is_callback = url.scheme() == "youskill"
    && url.host_str() == Some("auth")
    && url.path().trim_end_matches('/') == "/callback";
  if !is_callback {
    return None;
  }
  let mut callback = Callback::default();
  for (key, value) in url.query_pairs() {
    let value = Some(value.into_owned());
    match key.as_ref() {
      "code" => callback.code = value,
      "state" => callback.state = value,
      "error" => callback.error = value,
      _ => {},
    }
  }
  Some(callback)
}

fn parse<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, String> {
  serde_json::from_str(value).map_err(|error| error.to_string())
}

fn to_json<T: Serialize>(value: &T) -> Result<String, String> {
  serde_json::to_string(value).map_err(|error| error.to_string())
}

/// The account API over HTTP.
pub struct HttpAccountServer {
  client: Client,
  base_url: String,
  api_key: String,
}

impl HttpAccountServer {
  pub fn new(base_url: &str, api_key: &str) -> Result<Self, String> {
    let client = Client::builder()
      .timeout(Duration::from_secs(15))
      .build()
      .map_err(|error| format!("Failed to create HTTP client: {error}"))?;
    Ok(Self {
      client,
      base_url: base_url.trim_end_matches('/').to_string(),
      api_key: api_key.to_string(),
    })
  }

  /// The production server of the app.
  pub fn production() -> Result<Self, String> {
    Self::new(
      super::remote_service::API_BASE_URL,
      super::remote_service::API_KEY,
    )
  }

  fn post(&self, path: &str) -> reqwest::RequestBuilder {
    self
      .client
      .post(format!("{}/api/account/{path}", self.base_url))
      .header("X-API-Key", &self.api_key)
  }

  async fn session(
    &self,
    request: reqwest::RequestBuilder,
  ) -> Result<SessionResponse, ServerError> {
    let response = check(request.send().await).await?;
    response
      .json()
      .await
      .map_err(|error| ServerError::Failed(format!("Invalid session response: {error}")))
  }
}

/// The response of a successful request, or the error its status maps to.
pub(crate) async fn check(
  result: Result<reqwest::Response, reqwest::Error>,
) -> Result<reqwest::Response, ServerError> {
  let response =
    result.map_err(|error| ServerError::Failed(format!("Network request failed: {error}")))?;
  let status = response.status();
  if status.is_success() {
    return Ok(response);
  }
  let body = response.text().await.unwrap_or_default();
  Err(match status {
    StatusCode::UNAUTHORIZED => ServerError::Unauthorized,
    StatusCode::TOO_MANY_REQUESTS => {
      ServerError::Failed("Too many requests. Try again in a minute.".to_string())
    },
    status if status.is_client_error() => ServerError::Rejected(format!("{status} {body}")),
    status => ServerError::Failed(format!("Server error {status}")),
  })
}

impl AccountServer for HttpAccountServer {
  fn base_url(&self) -> &str {
    &self.base_url
  }

  async fn exchange(
    &self,
    code: &str,
    code_verifier: &str,
  ) -> Result<SessionResponse, ServerError> {
    let body = serde_json::json!({
      "code": code,
      "code_verifier": code_verifier,
      "redirect_uri": REDIRECT_URI,
    });
    self.session(self.post("token").json(&body)).await
  }

  async fn refresh(&self, refresh_token: &str) -> Result<SessionResponse, ServerError> {
    let body = serde_json::json!({ "refresh_token": refresh_token });
    self.session(self.post("refresh").json(&body)).await
  }

  async fn sign_out(&self, access_token: &str) -> Result<(), ServerError> {
    check(self.post("sign-out").bearer_auth(access_token).send().await)
      .await
      .map(|_| ())
  }

  async fn delete_account(&self, access_token: &str) -> Result<(), ServerError> {
    let request = self
      .client
      .delete(format!("{}/api/account", self.base_url))
      .header("X-API-Key", &self.api_key)
      .bearer_auth(access_token);
    check(request.send().await).await.map(|_| ())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::{FavoriteSnapshot, FavoriteStore};
  use std::collections::HashMap;
  use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
  use std::sync::{Arc, Mutex};

  const BASE: &str = "https://api.example.test";

  #[derive(Default)]
  struct MemorySecrets(Mutex<HashMap<String, String>>, AtomicBool);

  impl SecretStore for Arc<MemorySecrets> {
    fn load(&self, key: &str) -> Result<Option<String>, String> {
      Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn save(&self, key: &str, value: &str) -> Result<(), String> {
      if self.1.load(Ordering::SeqCst) {
        return Err("keychain locked".to_string());
      }
      self
        .0
        .lock()
        .unwrap()
        .insert(key.to_string(), value.to_string());
      Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
      self.0.lock().unwrap().remove(key);
      Ok(())
    }
  }

  /// In-memory account server with refresh token rotation and reuse detection.
  #[derive(Default)]
  struct FakeServer(Mutex<ServerState>);

  #[derive(Default)]
  struct ServerState {
    /// Code and its verifier challenge.
    codes: HashMap<String, String>,
    /// Refresh tokens that still work.
    valid_refresh: Vec<String>,
    revoked_access: Vec<String>,
    issued: u32,
    refreshes: u32,
    exchanges: Vec<(String, String)>,
    offline: bool,
    /// The account was deleted.
    deleted: bool,
    /// Tokens of `DELETE /account` calls.
    deletes: Vec<String>,
    /// Access tokens and their expiry, in the clock of the test.
    access: HashMap<String, i64>,
    clock: Arc<AtomicI64>,
  }

  impl FakeServer {
    fn with<T>(&self, f: impl FnOnce(&mut ServerState) -> T) -> T {
      f(&mut self.0.lock().unwrap())
    }
  }

  impl ServerState {
    fn issue(&mut self) -> SessionResponse {
      self.issued += 1;
      let refresh_token = format!("refresh-{}", self.issued);
      self.valid_refresh = vec![refresh_token.clone()];
      let access_token = format!("access-{}", self.issued);
      self.access.insert(
        access_token.clone(),
        self.clock.load(Ordering::SeqCst) + 900,
      );
      SessionResponse {
        access_token,
        expires_in: 900,
        refresh_token,
        account: Account {
          id: 12,
          display_name: Some("octo".to_string()),
          email: None,
          identities: vec![Identity {
            provider: "github".to_string(),
            display_name: Some("octo".to_string()),
            email: None,
          }],
        },
      }
    }
  }

  impl AccountServer for Arc<FakeServer> {
    fn base_url(&self) -> &str {
      BASE
    }

    async fn exchange(
      &self,
      code: &str,
      code_verifier: &str,
    ) -> Result<SessionResponse, ServerError> {
      self.with(|state| {
        state
          .exchanges
          .push((code.to_string(), code_verifier.to_string()));
        match state.codes.remove(code) {
          Some(challenge) if challenge == code_challenge(code_verifier) => Ok(state.issue()),
          _ => Err(ServerError::Rejected("400 invalid_grant".to_string())),
        }
      })
    }

    async fn refresh(&self, refresh_token: &str) -> Result<SessionResponse, ServerError> {
      // Let concurrent callers run between the request and the response.
      tokio::time::sleep(Duration::from_millis(20)).await;
      self.with(|state| {
        if state.offline {
          return Err(ServerError::Failed("offline".to_string()));
        }
        state.refreshes += 1;
        if !state
          .valid_refresh
          .iter()
          .any(|token| token == refresh_token)
        {
          // Reuse of a rotated token revokes the session.
          state.valid_refresh.clear();
          return Err(ServerError::Unauthorized);
        }
        Ok(state.issue())
      })
    }

    async fn sign_out(&self, access_token: &str) -> Result<(), ServerError> {
      self.with(|state| {
        if state.offline {
          return Err(ServerError::Failed("offline".to_string()));
        }
        let now = state.clock.load(Ordering::SeqCst);
        match state.access.get(access_token) {
          Some(expires_at) if *expires_at > now => {},
          _ => return Err(ServerError::Unauthorized),
        }
        state.revoked_access.push(access_token.to_string());
        state.valid_refresh.clear();
        Ok(())
      })
    }

    async fn delete_account(&self, access_token: &str) -> Result<(), ServerError> {
      self.with(|state| {
        state.deletes.push(access_token.to_string());
        if state.offline {
          return Err(ServerError::Failed("offline".to_string()));
        }
        let now = state.clock.load(Ordering::SeqCst);
        match state.access.get(access_token) {
          Some(expires_at) if *expires_at > now && !state.deleted => {},
          _ => return Err(ServerError::Unauthorized),
        }
        state.deleted = true;
        state.valid_refresh.clear();
        Ok(())
      })
    }
  }

  struct Setup {
    server: Arc<FakeServer>,
    secrets: Arc<MemorySecrets>,
    clock: Arc<AtomicI64>,
    favorites: PathBuf,
    _dir: tempfile::TempDir,
  }

  impl Setup {
    fn new() -> Self {
      let dir = tempfile::tempdir().unwrap();
      let clock = Arc::new(AtomicI64::new(1_000));
      let server = Arc::<FakeServer>::default();
      server.with(|state| state.clock = clock.clone());
      Self {
        server,
        secrets: Arc::default(),
        clock,
        favorites: dir.path().join("favorites.json"),
        _dir: dir,
      }
    }

    /// A manager as after an app start: it reads the stored session.
    fn manager(&self) -> AccountManager<Arc<FakeServer>, Arc<MemorySecrets>> {
      let clock = self.clock.clone();
      AccountManager::new(
        self.server.clone(),
        self.secrets.clone(),
        self.favorites.clone(),
        Box::new(move || clock.load(Ordering::SeqCst)),
      )
    }

    fn advance(&self, secs: i64) {
      self.clock.fetch_add(secs, Ordering::SeqCst);
    }

    fn secret(&self, key: &str) -> Option<String> {
      self.secrets.load(key).unwrap()
    }

    /// Runs the browser part: the server accepts the challenge and redirects with a code.
    fn browser(&self, start: &str, code: &str) -> String {
      let url = Url::parse(start).unwrap();
      let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
      assert_eq!(query["code_challenge_method"], "S256");
      assert_eq!(query["redirect_uri"], REDIRECT_URI);
      self.server.with(|state| {
        state
          .codes
          .insert(code.to_string(), query["code_challenge"].clone())
      });
      format!("{REDIRECT_URI}?code={code}&state={}", query["state"])
    }
  }

  fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()
      .unwrap()
      .block_on(future)
  }

  fn signed_in(setup: &Setup) -> AccountManager<Arc<FakeServer>, Arc<MemorySecrets>> {
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    assert!(matches!(
      block_on(manager.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::SignedIn(_)
    ));
    manager
  }

  #[test]
  fn the_start_url_has_an_s256_challenge_of_the_verifier_and_a_random_state() {
    let setup = Setup::new();
    let manager = setup.manager();
    let first = manager.begin_sign_in(Provider::Apple).unwrap();
    let pending: PendingSignIn = parse(&setup.secret(PENDING_KEY).unwrap()).unwrap();
    let second = manager.begin_sign_in(Provider::Apple).unwrap();

    let url = Url::parse(&first).unwrap();
    assert_eq!(url.path(), "/api/account/sign-in/apple");
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
      query["code_challenge"],
      code_challenge(&pending.code_verifier)
    );
    assert_eq!(query["state"], pending.state);
    assert_eq!(pending.code_verifier.len(), 64);
    assert!(pending
      .code_verifier
      .chars()
      .all(|c| c.is_ascii_alphanumeric()));
    assert_ne!(first, second);
  }

  #[test]
  fn the_challenge_is_base64url_sha256_without_padding() {
    // RFC 7636 appendix B.
    assert_eq!(
      code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
      "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
  }

  #[test]
  fn a_callback_exchanges_code_and_verifier_and_stores_the_session_in_the_secret_store() {
    let setup = Setup::new();
    let manager = signed_in(&setup);

    let status = block_on(manager.status());
    assert_eq!(
      status.identity,
      Some(AccountIdentity {
        account_id: "12".to_string(),
        provider: Provider::Github,
        display_name: "octo".to_string(),
      })
    );
    let stored: StoredSession = parse(&setup.secret(SESSION_KEY).unwrap()).unwrap();
    assert_eq!(stored.refresh_token, "refresh-1");
    assert_eq!(stored.expires_at, 1_900);
    assert!(setup.secret(PENDING_KEY).is_none());

    // The session stays after a restart.
    let restarted = setup.manager();
    assert_eq!(block_on(restarted.status()), status);
    assert_eq!(block_on(restarted.access_token()).unwrap(), "access-1");
  }

  #[test]
  fn a_callback_that_starts_the_app_completes_the_pending_sign_in() {
    let setup = Setup::new();
    let start = setup.manager().begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    let started_by_link = setup.manager();
    assert!(matches!(
      block_on(started_by_link.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::SignedIn(_)
    ));
    // A second delivery of the same link does nothing.
    assert_eq!(
      block_on(started_by_link.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::Ignored
    );
    assert_eq!(setup.server.with(|state| state.exchanges.len()), 1);
  }

  #[test]
  fn the_same_link_twice_at_once_exchanges_once() {
    let setup = Setup::new();
    let manager = Arc::new(setup.manager());
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    let outcomes = block_on(async {
      let calls: Vec<_> = (0..2)
        .map(|_| {
          let manager = manager.clone();
          let link = link.clone();
          tokio::spawn(async move { manager.complete_sign_in(&link).await })
        })
        .collect();
      let mut outcomes = Vec::new();
      for call in calls {
        outcomes.push(call.await.unwrap().unwrap());
      }
      outcomes
    });
    assert!(outcomes.contains(&CallbackOutcome::Ignored));
    assert!(outcomes
      .iter()
      .any(|outcome| matches!(outcome, CallbackOutcome::SignedIn(_))));
    assert_eq!(setup.server.with(|state| state.exchanges.len()), 1);
  }

  #[test]
  fn a_wrong_state_is_an_error_and_does_not_exchange() {
    let setup = Setup::new();
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    let forged = format!("{REDIRECT_URI}?code=code-1&state=other");
    assert!(block_on(manager.complete_sign_in(&forged)).is_err());
    assert!(setup.server.with(|state| state.exchanges.is_empty()));
    // The real callback still completes.
    assert!(matches!(
      block_on(manager.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::SignedIn(_)
    ));
  }

  #[test]
  fn cancel_returns_to_signed_out_without_an_error() {
    let setup = Setup::new();
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Apple).unwrap();
    let state = Url::parse(&start)
      .unwrap()
      .query_pairs()
      .find(|(key, _)| key == "state")
      .unwrap()
      .1
      .into_owned();
    let cancel = format!("{REDIRECT_URI}?error=access_denied&state={state}");
    assert_eq!(
      block_on(manager.complete_sign_in(&cancel)).unwrap(),
      CallbackOutcome::Cancelled
    );
    assert_eq!(block_on(manager.status()).identity, None);
    assert!(setup.secret(PENDING_KEY).is_none());

    let start = manager.begin_sign_in(Provider::Apple).unwrap();
    let state = Url::parse(&start)
      .unwrap()
      .query_pairs()
      .find(|(key, _)| key == "state")
      .unwrap()
      .1
      .into_owned();
    let failure = format!("{REDIRECT_URI}?error=server_error&state={state}");
    let error = block_on(manager.complete_sign_in(&failure)).unwrap_err();
    assert!(error.contains("server_error"));
  }

  #[test]
  fn a_rejected_or_late_code_is_an_error() {
    let setup = Setup::new();
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    setup.server.with(|state| state.codes.clear());
    assert!(block_on(manager.complete_sign_in(&link)).is_err());
    assert_eq!(block_on(manager.status()).identity, None);

    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-2");
    setup.advance(PENDING_TTL_SECS + 1);
    assert!(block_on(manager.complete_sign_in(&link)).is_err());
    assert_eq!(setup.server.with(|state| state.exchanges.len()), 1);
  }

  #[test]
  fn other_links_are_ignored() {
    let setup = Setup::new();
    let manager = setup.manager();
    manager.begin_sign_in(Provider::Github).unwrap();
    for link in [
      "youskill://other?code=1",
      "https://auth/callback?code=1",
      "x",
    ] {
      assert_eq!(
        block_on(manager.complete_sign_in(link)).unwrap(),
        CallbackOutcome::Ignored
      );
    }
    assert!(setup.secret(PENDING_KEY).is_some());
  }

  #[test]
  fn the_access_token_refreshes_near_expiry_and_keeps_the_rotated_refresh_token() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.advance(900 - REFRESH_MARGIN_SECS - 1);
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-1");

    setup.advance(1);
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-2");
    let stored: StoredSession = parse(&setup.secret(SESSION_KEY).unwrap()).unwrap();
    assert_eq!(stored.refresh_token, "refresh-2");

    // After a restart the rotated token refreshes; the old one would revoke the session.
    setup.advance(900);
    assert_eq!(
      block_on(setup.manager().access_token()).unwrap(),
      "access-3"
    );
    assert_eq!(setup.server.with(|state| state.refreshes), 2);
  }

  #[test]
  fn concurrent_callers_share_one_refresh() {
    let setup = Setup::new();
    let manager = Arc::new(signed_in(&setup));
    setup.advance(900);
    let tokens = block_on(async {
      let calls = (0..5).map(|_| {
        let manager = manager.clone();
        tokio::spawn(async move { manager.access_token().await })
      });
      let mut tokens = Vec::new();
      for call in calls.collect::<Vec<_>>() {
        tokens.push(call.await.unwrap().unwrap());
      }
      tokens
    });
    assert!(tokens.iter().all(|token| token == "access-2"));
    assert_eq!(setup.server.with(|state| state.refreshes), 1);
  }

  #[test]
  fn a_rejected_token_refreshes_once_for_all_callers() {
    let setup = Setup::new();
    let manager = Arc::new(signed_in(&setup));
    let tokens = block_on(async {
      let calls: Vec<_> = (0..2)
        .map(|_| {
          let manager = manager.clone();
          tokio::spawn(async move { manager.refresh_rejected("access-1").await })
        })
        .collect();
      let mut tokens = Vec::new();
      for call in calls {
        tokens.push(call.await.unwrap().unwrap());
      }
      tokens
    });
    assert_eq!(tokens, ["access-2", "access-2"]);
    assert_eq!(setup.server.with(|state| state.refreshes), 1);
  }

  #[test]
  fn unauthorized_refresh_signs_out_and_keeps_local_favorites() {
    let setup = Setup::new();
    let mut store = FavoriteStore::default();
    store
      .set(
        "42",
        true,
        Some(FavoriteSnapshot {
          name: "pdf".to_string(),
          source: "team/tools".to_string(),
          description: None,
        }),
        1,
      )
      .unwrap();
    favorites_service::save_store(&setup.favorites, &store).unwrap();
    let manager = signed_in(&setup);

    setup.server.with(|state| state.valid_refresh.clear());
    setup.advance(900);
    assert_eq!(
      block_on(manager.access_token()),
      Err(AccountError::SignedOut)
    );
    assert_eq!(block_on(manager.status()).identity, None);
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(
      favorites_service::load_store(&setup.favorites).unwrap(),
      store
    );
  }

  #[test]
  fn a_failed_refresh_keeps_the_session() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.server.with(|state| state.offline = true);
    setup.advance(900);
    assert!(matches!(
      block_on(manager.access_token()),
      Err(AccountError::Failed(_))
    ));
    assert!(block_on(manager.status()).identity.is_some());
    setup.server.with(|state| state.offline = false);
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-2");
  }

  #[test]
  fn sign_out_revokes_deletes_tokens_and_clears_favorites_and_cursor() {
    let setup = Setup::new();
    let mut store = FavoriteStore::default();
    store.begin_account("12");
    store.cursor = Some("8".to_string());
    store
      .set(
        "42",
        true,
        Some(FavoriteSnapshot {
          name: "pdf".to_string(),
          source: "team/tools".to_string(),
          description: None,
        }),
        1,
      )
      .unwrap();
    favorites_service::save_store(&setup.favorites, &store).unwrap();
    let manager = signed_in(&setup);

    block_on(manager.sign_out()).unwrap();

    assert_eq!(
      setup.server.with(|state| state.revoked_access.clone()),
      ["access-1"]
    );
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
    assert_eq!(
      favorites_service::load_store(&setup.favorites).unwrap(),
      FavoriteStore::default()
    );
    assert_eq!(
      block_on(setup.manager().access_token()),
      Err(AccountError::SignedOut)
    );
  }

  #[test]
  fn sign_out_finishes_locally_when_the_server_is_unreachable() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.server.with(|state| state.offline = true);
    block_on(manager.sign_out()).unwrap();
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
  }

  #[test]
  fn sign_out_after_the_access_token_expired_refreshes_and_revokes() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.advance(900);
    block_on(manager.sign_out()).unwrap();
    assert_eq!(
      setup.server.with(|state| state.revoked_access.clone()),
      ["access-2"]
    );
    assert!(setup.server.with(|state| state.valid_refresh.is_empty()));
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
  }

  #[test]
  fn sign_out_of_a_revoked_session_clears_locally_without_a_revoke() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.server.with(|state| state.valid_refresh.clear());
    setup.advance(900);
    block_on(manager.sign_out()).unwrap();
    assert!(setup.server.with(|state| state.revoked_access.is_empty()));
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
  }

  #[test]
  fn a_failed_keychain_save_keeps_the_rotated_refresh_token_in_memory() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.secrets.1.store(true, Ordering::SeqCst);
    setup.advance(900);
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-2");
    // The next refresh uses the rotated token; the old one would revoke the session.
    setup.advance(900);
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-3");
    assert!(block_on(manager.status()).identity.is_some());
  }

  #[test]
  fn a_failed_keychain_save_at_sign_in_still_signs_in() {
    let setup = Setup::new();
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let link = setup.browser(&start, "code-1");
    setup.secrets.1.store(true, Ordering::SeqCst);
    assert!(matches!(
      block_on(manager.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::SignedIn(_)
    ));
    assert_eq!(block_on(manager.access_token()).unwrap(), "access-1");
  }

  #[test]
  fn a_cancel_of_a_replaced_sign_in_is_ignored_and_keeps_the_new_one() {
    let setup = Setup::new();
    let manager = setup.manager();
    let old = manager.begin_sign_in(Provider::Github).unwrap();
    let old_state = state_of(&old);
    let start = manager.begin_sign_in(Provider::Github).unwrap();
    let cancel = format!("{REDIRECT_URI}?error=access_denied&state={old_state}");
    assert_eq!(
      block_on(manager.complete_sign_in(&cancel)).unwrap(),
      CallbackOutcome::Ignored
    );
    let link = setup.browser(&start, "code-1");
    assert!(matches!(
      block_on(manager.complete_sign_in(&link)).unwrap(),
      CallbackOutcome::SignedIn(_)
    ));
  }

  #[test]
  fn a_late_cancel_is_still_a_cancel() {
    let setup = Setup::new();
    let manager = setup.manager();
    let start = manager.begin_sign_in(Provider::Apple).unwrap();
    setup.advance(PENDING_TTL_SECS + 1);
    let cancel = format!(
      "{REDIRECT_URI}?error=access_denied&state={}",
      state_of(&start)
    );
    assert_eq!(
      block_on(manager.complete_sign_in(&cancel)).unwrap(),
      CallbackOutcome::Cancelled
    );
    assert!(setup.secret(PENDING_KEY).is_none());
  }

  /// Favorites of an account with a sync cursor, saved before the sign-in.
  fn synced_favorites(setup: &Setup) -> FavoriteStore {
    let mut store = FavoriteStore::default();
    store.begin_account("12");
    store.cursor = Some("8".to_string());
    store
      .set(
        "42",
        true,
        Some(FavoriteSnapshot {
          name: "pdf".to_string(),
          source: "team/tools".to_string(),
          description: None,
        }),
        1,
      )
      .unwrap();
    favorites_service::save_store(&setup.favorites, &store).unwrap();
    store
  }

  #[test]
  fn delete_account_deletes_on_the_server_then_tokens_favorites_and_cursor() {
    let setup = Setup::new();
    synced_favorites(&setup);
    let manager = signed_in(&setup);

    block_on(manager.delete_account()).unwrap();

    assert_eq!(
      setup.server.with(|state| state.deletes.clone()),
      ["access-1"]
    );
    assert!(setup.server.with(|state| state.deleted));
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
    assert_eq!(
      favorites_service::load_store(&setup.favorites).unwrap(),
      FavoriteStore::default()
    );
    assert_eq!(
      block_on(setup.manager().access_token()),
      Err(AccountError::SignedOut)
    );
  }

  #[test]
  fn a_failed_delete_changes_nothing_local_and_can_be_retried() {
    let setup = Setup::new();
    let store = synced_favorites(&setup);
    let manager = signed_in(&setup);
    let session = setup.secret(SESSION_KEY);
    setup.server.with(|state| state.offline = true);

    assert!(matches!(
      block_on(manager.delete_account()),
      Err(AccountError::Failed(_))
    ));
    assert!(!setup.server.with(|state| state.deleted));
    assert_eq!(setup.secret(SESSION_KEY), session);
    assert!(block_on(manager.status()).identity.is_some());
    assert_eq!(
      favorites_service::load_store(&setup.favorites).unwrap(),
      store
    );

    setup.server.with(|state| state.offline = false);
    block_on(manager.delete_account()).unwrap();
    assert!(setup.server.with(|state| state.deleted));
    assert_eq!(block_on(manager.status()).identity, None);
  }

  #[test]
  fn delete_account_refreshes_an_expired_access_token_first() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    setup.advance(900);
    block_on(manager.delete_account()).unwrap();
    assert_eq!(
      setup.server.with(|state| state.deletes.clone()),
      ["access-2"]
    );
    assert!(setup.server.with(|state| state.deleted));
  }

  #[test]
  fn a_401_on_delete_refreshes_once_and_retries() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    // The server revoked the access token early; the refresh token still works.
    setup.server.with(|state| state.access.clear());
    block_on(manager.delete_account()).unwrap();
    assert_eq!(
      setup.server.with(|state| state.deletes.clone()),
      ["access-1", "access-2"]
    );
    assert_eq!(block_on(manager.status()).identity, None);
  }

  #[test]
  fn a_401_on_delete_that_a_refresh_cannot_fix_signs_out_and_keeps_favorites() {
    let setup = Setup::new();
    let store = synced_favorites(&setup);
    let manager = signed_in(&setup);
    setup.server.with(|state| {
      state.access.clear();
      state.valid_refresh.clear();
    });

    assert_eq!(
      block_on(manager.delete_account()),
      Err(AccountError::SignedOut)
    );
    assert_eq!(setup.server.with(|state| state.deletes.len()), 1);
    assert!(!setup.server.with(|state| state.deleted));
    assert!(setup.secret(SESSION_KEY).is_none());
    assert_eq!(block_on(manager.status()).identity, None);
    assert_eq!(
      favorites_service::load_store(&setup.favorites).unwrap(),
      store
    );
  }

  #[test]
  fn a_second_401_after_the_refresh_signs_out() {
    let setup = Setup::new();
    let manager = signed_in(&setup);
    // Every access token gives 401, also the refreshed one.
    setup.server.with(|state| state.deleted = true);
    assert_eq!(
      block_on(manager.delete_account()),
      Err(AccountError::SignedOut)
    );
    assert_eq!(setup.server.with(|state| state.deletes.len()), 2);
    assert!(setup.secret(SESSION_KEY).is_none());
  }

  #[test]
  fn delete_account_when_signed_out_is_signed_out() {
    let setup = Setup::new();
    assert_eq!(
      block_on(setup.manager().delete_account()),
      Err(AccountError::SignedOut)
    );
    assert!(setup.server.with(|state| state.deletes.is_empty()));
  }

  fn state_of(start: &str) -> String {
    Url::parse(start)
      .unwrap()
      .query_pairs()
      .find(|(key, _)| key == "state")
      .unwrap()
      .1
      .into_owned()
  }

  /// One HTTP exchange against a local socket: returns the raw request.
  fn http_once(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
      let (mut stream, _) = listener.accept().unwrap();
      stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
      let mut request = Vec::new();
      let mut buffer = [0; 4096];
      loop {
        let count = stream.read(&mut buffer).unwrap();
        request.extend_from_slice(&buffer[..count]);
        let text = String::from_utf8_lossy(&request).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
          let length = text
            .lines()
            .find_map(|line| {
              line
                .to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|value| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
          if request.len() >= end + 4 + length || count == 0 {
            break;
          }
        }
      }
      write!(
        stream,
        "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
      )
      .unwrap();
      String::from_utf8(request).unwrap()
    });
    (format!("http://{address}"), handle)
  }

  const SESSION_BODY: &str = r#"{"access_token":"a","token_type":"Bearer","expires_in":900,"refresh_token":"r","account":{"id":12,"display_name":"octo","email":null,"identities":[{"provider":"apple","display_name":null,"email":"o@example.test"}],"created_at":"2026-11-01T10:00:00Z"}}"#;

  #[test]
  fn the_http_exchange_follows_the_account_api() {
    let (base, request) = http_once(200, SESSION_BODY);
    let server = HttpAccountServer::new(&base, "key").unwrap();
    let response = block_on(server.exchange("c", "v")).unwrap();
    assert_eq!(response.refresh_token, "r");
    assert_eq!(response.account.identities[0].provider, "apple");

    let request = request.join().unwrap();
    assert!(request.starts_with("POST /api/account/token "));
    assert!(request.to_ascii_lowercase().contains("x-api-key: key"));
    let body: serde_json::Value =
      serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
      body,
      serde_json::json!({"code": "c", "code_verifier": "v", "redirect_uri": REDIRECT_URI})
    );
  }

  #[test]
  fn the_http_refresh_maps_401_to_unauthorized() {
    let (base, request) = http_once(401, r#"{"detail":"invalid"}"#);
    let server = HttpAccountServer::new(&base, "key").unwrap();
    assert_eq!(
      block_on(server.refresh("r")),
      Err(ServerError::Unauthorized)
    );
    assert!(request
      .join()
      .unwrap()
      .starts_with("POST /api/account/refresh "));
  }

  #[test]
  fn the_http_sign_out_sends_the_bearer_token() {
    let (base, request) = http_once(204, "");
    let server = HttpAccountServer::new(&base, "key").unwrap();
    block_on(server.sign_out("a")).unwrap();
    let request = request.join().unwrap().to_ascii_lowercase();
    assert!(request.starts_with("post /api/account/sign-out "));
    assert!(request.contains("authorization: bearer a"));
  }

  #[test]
  fn the_http_delete_account_sends_delete_with_key_and_bearer_token() {
    let (base, request) = http_once(204, "");
    let server = HttpAccountServer::new(&base, "key").unwrap();
    block_on(server.delete_account("a")).unwrap();
    let request = request.join().unwrap().to_ascii_lowercase();
    assert!(request.starts_with("delete /api/account "));
    assert!(request.contains("x-api-key: key"));
    assert!(request.contains("authorization: bearer a"));

    let (base, _request) = http_once(503, "");
    let server = HttpAccountServer::new(&base, "key").unwrap();
    assert!(matches!(
      block_on(server.delete_account("a")),
      Err(ServerError::Failed(_))
    ));
  }

  #[test]
  fn the_identity_name_falls_back_to_the_account_name_then_the_email() {
    let mut session = StoredSession {
      access_token: String::new(),
      refresh_token: String::new(),
      expires_at: 0,
      provider: Provider::Apple,
      account: serde_json::from_str::<SessionResponse>(SESSION_BODY)
        .unwrap()
        .account,
    };
    assert_eq!(identity(&session).display_name, "octo");
    session.account.display_name = None;
    assert_eq!(identity(&session).display_name, "o@example.test");
  }
}
