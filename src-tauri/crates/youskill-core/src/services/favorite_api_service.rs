//! The favorite API of the account server over HTTP (`youskill-web/docs/account-api.md`),
//! as the `FavoriteServer` of the sync engine.
//!
//! Each request sends the app API key and the access token of the account session. A `401`
//! refreshes the session once and retries; when the refresh is rejected too, the session is
//! gone and the sync returns `Unauthorized`.

use crate::models::{FavoriteRecord, FavoriteSnapshot};
use crate::services::account_service::{
  check, AccountError, AccountManager, AccountServer, SecretStore, ServerError,
};
use crate::services::favorite_sync_service::{
  FavoriteServer, PullPage, PushResult, RejectReason, SyncError,
};
use chrono::{DateTime, SecondsFormat};
use reqwest::{Client, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Records per pull page.
const PULL_LIMIT: usize = 500;

/// HTTP client and address of the favorite API.
pub struct FavoriteApi {
  client: Client,
  base_url: String,
  api_key: String,
}

impl FavoriteApi {
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

  /// The favorite server of the signed-in account of `account`.
  pub fn server<'a, S, K>(
    &'a self,
    account: &'a AccountManager<S, K>,
  ) -> AccountFavorites<'a, S, K> {
    AccountFavorites { api: self, account }
  }

  fn url(&self) -> String {
    format!("{}/api/account/favorites", self.base_url)
  }
}

/// `FavoriteServer` over HTTP with the session of one account manager.
pub struct AccountFavorites<'a, S, K> {
  api: &'a FavoriteApi,
  account: &'a AccountManager<S, K>,
}

impl<S, K> AccountFavorites<'_, S, K>
where
  S: AccountServer + Sync,
  K: SecretStore + Sync,
{
  async fn send<T: DeserializeOwned>(
    &self,
    request: impl Fn(&str) -> RequestBuilder,
  ) -> Result<T, SyncError> {
    let token = self.account.access_token().await.map_err(account_error)?;
    let response = match check(request(&token).send().await).await {
      Err(ServerError::Unauthorized) => {
        let token = self
          .account
          .refresh_rejected(&token)
          .await
          .map_err(account_error)?;
        check(request(&token).send().await).await
      },
      other => other,
    }
    .map_err(|error| match error {
      // The session refreshed but the server still rejects it; keep it and try later.
      ServerError::Unauthorized => {
        SyncError::Failed("The server rejected the refreshed session".to_string())
      },
      ServerError::Rejected(message) | ServerError::Failed(message) => SyncError::Failed(message),
    })?;
    response
      .json()
      .await
      .map_err(|error| SyncError::Failed(format!("Invalid favorites response: {error}")))
  }
}

fn account_error(error: AccountError) -> SyncError {
  match error {
    AccountError::SignedOut => SyncError::Unauthorized,
    AccountError::Failed(message) => SyncError::Failed(message),
  }
}

impl<S, K> FavoriteServer for AccountFavorites<'_, S, K>
where
  S: AccountServer + Sync,
  K: SecretStore + Sync,
{
  async fn push(&self, changes: &[FavoriteRecord]) -> Result<Vec<PushResult>, SyncError> {
    let mut results = Vec::new();
    let mut body = PushRequest {
      changes: Vec::new(),
    };
    for change in changes {
      match WireChange::from_record(change) {
        Some(wire) => body.changes.push(wire),
        // The server cannot store it; drop it here so it does not block the queue.
        None => results.push(PushResult::Rejected {
          skill_id: change.skill_id.clone(),
          reason: RejectReason::Other("invalid_skill_id".to_string()),
          record: None,
        }),
      }
    }
    if body.changes.is_empty() {
      return Ok(results);
    }
    let response: PushResponse = self
      .send(|token| {
        self
          .api
          .client
          .post(self.api.url())
          .header("X-API-Key", &self.api.api_key)
          .bearer_auth(token)
          .json(&body)
      })
      .await?;
    for result in response.results {
      results.push(result.into_push_result()?);
    }
    Ok(results)
  }

  async fn pull(&self, cursor: Option<&str>) -> Result<PullPage, SyncError> {
    let mut query = vec![("limit", PULL_LIMIT.to_string())];
    if let Some(cursor) = cursor {
      query.push(("cursor", cursor.to_string()));
    }
    let response: PullResponse = self
      .send(|token| {
        self
          .api
          .client
          .get(self.api.url())
          .header("X-API-Key", &self.api.api_key)
          .bearer_auth(token)
          .query(&query)
      })
      .await?;
    Ok(PullPage {
      records: response
        .favorites
        .into_iter()
        .map(WireRecord::into_record)
        .collect::<Result<_, _>>()?,
      cursor: response.cursor.map(|cursor| cursor.to_string()),
      has_more: response.has_more,
    })
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireState {
  Favorited,
  Removed,
}

#[derive(Debug, Serialize)]
struct PushRequest {
  changes: Vec<WireChange>,
}

#[derive(Debug, Serialize)]
struct WireChange {
  skill_id: i64,
  state: WireState,
  updated_at: String,
  /// Required for an add; a removal has none.
  #[serde(skip_serializing_if = "Option::is_none")]
  snapshot: Option<FavoriteSnapshot>,
}

impl WireChange {
  /// `None` when the skill id is not a marketplace id.
  fn from_record(record: &FavoriteRecord) -> Option<Self> {
    let updated_at = DateTime::from_timestamp_millis(record.updated_at)?;
    Some(Self {
      skill_id: record.skill_id.parse().ok()?,
      state: if record.favorited {
        WireState::Favorited
      } else {
        WireState::Removed
      },
      updated_at: updated_at.to_rfc3339_opts(SecondsFormat::Millis, true),
      snapshot: record.favorited.then(|| record.snapshot.clone()),
    })
  }
}

#[derive(Debug, Deserialize)]
struct WireRecord {
  skill_id: i64,
  state: WireState,
  updated_at: String,
  #[serde(default)]
  snapshot: Option<FavoriteSnapshot>,
}

impl WireRecord {
  fn into_record(self) -> Result<FavoriteRecord, SyncError> {
    let updated_at = DateTime::parse_from_rfc3339(&self.updated_at)
      .map_err(|error| SyncError::Failed(format!("Invalid favorite time: {error}")))?
      .timestamp_millis();
    Ok(FavoriteRecord {
      skill_id: self.skill_id.to_string(),
      favorited: self.state == WireState::Favorited,
      updated_at,
      // A removal has no snapshot; it is not shown.
      snapshot: self.snapshot.unwrap_or(FavoriteSnapshot {
        name: String::new(),
        source: String::new(),
        description: None,
      }),
    })
  }
}

#[derive(Debug, Deserialize)]
struct PushResponse {
  results: Vec<WireResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireStatus {
  Applied,
  Ignored,
  Rejected,
}

#[derive(Debug, Deserialize)]
struct WireResult {
  skill_id: i64,
  status: WireStatus,
  #[serde(default)]
  error: Option<String>,
  #[serde(default)]
  record: Option<WireRecord>,
}

impl WireResult {
  fn into_push_result(self) -> Result<PushResult, SyncError> {
    let record = self.record.map(WireRecord::into_record).transpose()?;
    match self.status {
      WireStatus::Applied | WireStatus::Ignored => {
        record.map(PushResult::Applied).ok_or_else(|| {
          SyncError::Failed(format!(
            "No record for skill {} in the push result",
            self.skill_id
          ))
        })
      },
      WireStatus::Rejected => Ok(PushResult::Rejected {
        skill_id: self.skill_id.to_string(),
        reason: match self.error.as_deref() {
          Some("favorite_limit_reached") => RejectReason::LimitReached,
          other => RejectReason::Other(other.unwrap_or("rejected").to_string()),
        },
        record,
      }),
    }
  }
}

#[derive(Debug, Deserialize)]
struct PullResponse {
  favorites: Vec<WireRecord>,
  #[serde(default)]
  cursor: Option<i64>,
  has_more: bool,
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use crate::services::account_service::{Account, SessionResponse};
  use std::collections::HashMap;
  use std::io::{Read, Write};
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::{Arc, Mutex};
  use std::thread::JoinHandle;

  /// 2026-11-01T10:00:00Z.
  const TEN: i64 = 1_793_527_200_000;

  // Payloads recorded from the contract in `youskill-web/docs/account-api.md`.
  const PUSH_RESPONSE: &str = r#"{
    "results": [
      {"skill_id": 42, "status": "applied", "error": null,
       "record": {"skill_id": 42, "state": "favorited", "updated_at": "2026-11-01T10:00:00Z", "change_seq": 7,
                  "snapshot": {"name": "pdf", "source": "owner/repo", "description": "..."}}},
      {"skill_id": 9, "status": "ignored", "error": null,
       "record": {"skill_id": 9, "state": "favorited", "updated_at": "2026-11-01T10:02:00Z", "change_seq": 9,
                  "snapshot": {"name": "docx", "source": "owner/repo", "description": null}}},
      {"skill_id": 77, "status": "rejected", "error": "favorite_limit_reached", "record": null}
    ]
  }"#;

  const PULL_RESPONSE: &str = r#"{
    "favorites": [
      {"skill_id": 42, "state": "favorited", "updated_at": "2026-11-01T18:00:00+08:00", "change_seq": 7,
       "snapshot": {"name": "pdf", "source": "owner/repo", "description": "..."}},
      {"skill_id": 9, "state": "removed", "updated_at": "2026-11-01T10:01:00Z", "change_seq": 8, "snapshot": null}
    ],
    "cursor": 8,
    "has_more": false
  }"#;

  #[derive(Default)]
  pub(crate) struct MemorySecrets(pub Mutex<HashMap<String, String>>);

  impl SecretStore for Arc<MemorySecrets> {
    fn load(&self, key: &str) -> Result<Option<String>, String> {
      Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn save(&self, key: &str, value: &str) -> Result<(), String> {
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

  /// Account server whose refresh gives `access-2`, or `401` when `revoked`.
  #[derive(Default)]
  pub(crate) struct Sessions {
    pub revoked: AtomicBool,
    pub signed_out: AtomicBool,
    pub deleted: AtomicBool,
  }

  impl AccountServer for Arc<Sessions> {
    fn base_url(&self) -> &str {
      "https://api.example.test"
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<SessionResponse, ServerError> {
      Err(ServerError::Rejected("unused".to_string()))
    }
    async fn refresh(&self, _: &str) -> Result<SessionResponse, ServerError> {
      if self.revoked.load(Ordering::SeqCst) {
        return Err(ServerError::Unauthorized);
      }
      Ok(SessionResponse {
        access_token: "access-2".to_string(),
        expires_in: 900,
        refresh_token: "refresh-2".to_string(),
        account: Account {
          id: 12,
          display_name: None,
          email: None,
          identities: Vec::new(),
        },
      })
    }
    async fn sign_out(&self, _: &str) -> Result<(), ServerError> {
      self.signed_out.store(true, Ordering::SeqCst);
      Ok(())
    }
    async fn delete_account(&self, _: &str) -> Result<(), ServerError> {
      self.deleted.store(true, Ordering::SeqCst);
      Ok(())
    }
  }

  pub(crate) type TestAccount = AccountManager<Arc<Sessions>, Arc<MemorySecrets>>;

  /// An account manager signed in to account 12 with `access-1`.
  pub(crate) fn signed_in(
    favorites: std::path::PathBuf,
  ) -> (TestAccount, Arc<Sessions>, Arc<MemorySecrets>) {
    let secrets = Arc::<MemorySecrets>::default();
    secrets
      .save(
        "session",
        r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_at":1000000,"provider":"github","account":{"id":12}}"#,
      )
      .unwrap();
    let sessions = Arc::<Sessions>::default();
    let account = AccountManager::new(
      sessions.clone(),
      secrets.clone(),
      favorites,
      Box::new(|| 1_000),
    );
    (account, sessions, secrets)
  }

  /// Serves `responses` in order on a local socket and returns the raw requests.
  fn http_server(responses: Vec<(u16, &'static str)>) -> (String, JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
      let mut requests = Vec::new();
      for (status, body) in responses {
        let (mut stream, _) = listener.accept().unwrap();
        stream
          .set_read_timeout(Some(Duration::from_secs(5)))
          .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 8192];
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
        requests.push(String::from_utf8(request).unwrap());
      }
      requests
    });
    (format!("http://{address}"), handle)
  }

  fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
      .enable_all()
      .build()
      .unwrap()
      .block_on(future)
  }

  fn body(request: &str) -> serde_json::Value {
    serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
  }

  fn snapshot(name: &str, description: Option<&str>) -> FavoriteSnapshot {
    FavoriteSnapshot {
      name: name.to_string(),
      source: "owner/repo".to_string(),
      description: description.map(str::to_string),
    }
  }

  fn record(skill_id: &str, favorited: bool, updated_at: i64, name: &str) -> FavoriteRecord {
    FavoriteRecord {
      skill_id: skill_id.to_string(),
      favorited,
      updated_at,
      snapshot: snapshot(name, None),
    }
  }

  #[test]
  fn a_push_sends_key_bearer_and_contract_changes_and_maps_the_results() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    let (base, requests) = http_server(vec![(200, PUSH_RESPONSE)]);
    let api = FavoriteApi::new(&base, "key").unwrap();
    let mut add = record("42", true, TEN, "pdf");
    add.snapshot.description = Some("...".to_string());
    let changes = [
      add.clone(),
      record("9", false, TEN + 60_000, "docx"),
      record("77", true, TEN + 1, "xlsx"),
    ];

    let results = block_on(api.server(&account).push(&changes)).unwrap();

    assert_eq!(
      results,
      [
        PushResult::Applied(add),
        PushResult::Applied(record("9", true, TEN + 120_000, "docx")),
        PushResult::Rejected {
          skill_id: "77".to_string(),
          reason: RejectReason::LimitReached,
          record: None,
        },
      ]
    );
    let request = &requests.join().unwrap()[0];
    assert!(request.starts_with("POST /api/account/favorites "));
    let lower = request.to_ascii_lowercase();
    assert!(lower.contains("x-api-key: key"));
    assert!(lower.contains("authorization: bearer access-1"));
    assert_eq!(
      body(request),
      serde_json::json!({"changes": [
        {"skill_id": 42, "state": "favorited", "updated_at": "2026-11-01T10:00:00.000Z",
         "snapshot": {"name": "pdf", "source": "owner/repo", "description": "..."}},
        {"skill_id": 9, "state": "removed", "updated_at": "2026-11-01T10:01:00.000Z"},
        {"skill_id": 77, "state": "favorited", "updated_at": "2026-11-01T10:00:00.001Z",
         "snapshot": {"name": "xlsx", "source": "owner/repo", "description": null}},
      ]})
    );
  }

  #[test]
  fn a_pull_sends_the_cursor_and_maps_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    let (base, requests) = http_server(vec![(200, PULL_RESPONSE)]);
    let api = FavoriteApi::new(&base, "key").unwrap();

    let page = block_on(api.server(&account).pull(Some("6"))).unwrap();

    let mut pdf = record("42", true, TEN, "pdf");
    pdf.snapshot.description = Some("...".to_string());
    assert_eq!(page.records[0], pdf);
    assert!(!page.records[1].favorited);
    assert_eq!(page.records[1].skill_id, "9");
    assert_eq!(page.records[1].updated_at, TEN + 60_000);
    assert_eq!(page.cursor.as_deref(), Some("8"));
    assert!(!page.has_more);
    let request = &requests.join().unwrap()[0];
    assert!(request.starts_with("GET /api/account/favorites?limit=500&cursor=6 "));
    assert!(request
      .to_ascii_lowercase()
      .contains("authorization: bearer access-1"));
  }

  #[test]
  fn the_first_pull_has_no_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    let (base, requests) = http_server(vec![(
      200,
      r#"{"favorites": [], "cursor": null, "has_more": false}"#,
    )]);
    let api = FavoriteApi::new(&base, "key").unwrap();
    let page = block_on(api.server(&account).pull(None)).unwrap();
    assert_eq!(page.cursor, None);
    assert!(requests.join().unwrap()[0].starts_with("GET /api/account/favorites?limit=500 "));
  }

  #[test]
  fn a_401_refreshes_once_and_retries_with_the_new_token() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    let (base, requests) = http_server(vec![(401, "{}"), (200, PULL_RESPONSE)]);
    let api = FavoriteApi::new(&base, "key").unwrap();

    assert!(block_on(api.server(&account).pull(None)).is_ok());

    let requests = requests.join().unwrap();
    assert!(requests[0]
      .to_ascii_lowercase()
      .contains("authorization: bearer access-1"));
    assert!(requests[1]
      .to_ascii_lowercase()
      .contains("authorization: bearer access-2"));
  }

  #[test]
  fn a_401_with_a_revoked_session_is_unauthorized_and_signs_out() {
    let dir = tempfile::tempdir().unwrap();
    let (account, sessions, _) = signed_in(dir.path().join("favorites.json"));
    sessions.revoked.store(true, Ordering::SeqCst);
    let (base, requests) = http_server(vec![(401, "{}")]);
    let api = FavoriteApi::new(&base, "key").unwrap();

    assert_eq!(
      block_on(api.server(&account).push(&[record("1", true, TEN, "pdf")])),
      Err(SyncError::Unauthorized)
    );
    assert_eq!(requests.join().unwrap().len(), 1);
    assert_eq!(block_on(account.status()).identity, None);
  }

  #[test]
  fn server_and_rate_limit_errors_are_failures() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    let (base, requests) = http_server(vec![(429, "{}"), (503, "{}")]);
    let api = FavoriteApi::new(&base, "key").unwrap();
    let server = api.server(&account);
    assert!(matches!(
      block_on(server.pull(None)),
      Err(SyncError::Failed(_))
    ));
    assert!(matches!(
      block_on(server.pull(None)),
      Err(SyncError::Failed(_))
    ));
    requests.join().unwrap();
  }

  #[test]
  fn a_change_without_a_marketplace_id_is_rejected_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let (account, _, _) = signed_in(dir.path().join("favorites.json"));
    // No server listens: a request would fail.
    let api = FavoriteApi::new("http://127.0.0.1:9", "key").unwrap();
    let results = block_on(
      api
        .server(&account)
        .push(&[record("abc", true, TEN, "pdf")]),
    )
    .unwrap();
    assert!(matches!(
      &results[..],
      [PushResult::Rejected { skill_id, record: None, .. }] if skill_id == "abc"
    ));
  }

  #[test]
  fn signed_out_is_unauthorized_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let account = AccountManager::new(
      Arc::<Sessions>::default(),
      Arc::<MemorySecrets>::default(),
      dir.path().join("favorites.json"),
      Box::new(|| 1_000),
    );
    let api = FavoriteApi::new("http://127.0.0.1:9", "key").unwrap();
    assert_eq!(
      block_on(api.server(&account).pull(None)),
      Err(SyncError::Unauthorized)
    );
  }
}
