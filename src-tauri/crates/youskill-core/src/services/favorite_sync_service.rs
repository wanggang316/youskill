//! Favorite sync between this device and an account. It pushes pending changes, then pulls
//! the changes after the stored cursor. The server keeps the later write per skill and
//! returns the result, which replaces the local record.
//!
//! The engine does not know the transport. `FavoriteServer` is the boundary to the favorite
//! API of the server.

use crate::models::{FavoriteRecord, FavoriteStore};
use crate::services::account_service::{AccountManager, AccountServer, SecretStore};
use crate::services::favorites_service;
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

/// The server accepts at most this many changes in one push.
pub const PUSH_LIMIT: usize = 500;

/// Why the server did not apply a pushed change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
  /// The account has the maximum number of favorites.
  LimitReached,
  Other(String),
}

/// Result of one pushed change, in push order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
  /// The record the server keeps after last write wins: the pushed change when the server
  /// applied it, else the stored record that is as new or newer.
  Applied(FavoriteRecord),
  Rejected {
    skill_id: String,
    reason: RejectReason,
    /// The record the server keeps, if it has one.
    record: Option<FavoriteRecord>,
  },
}

/// One page of changes after a cursor, removals included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullPage {
  pub records: Vec<FavoriteRecord>,
  pub cursor: Option<String>,
  pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
  /// The session is not valid any more. The app moves to the signed-out state.
  Unauthorized,
  /// Network or server failure. Pending changes stay and retry on the next sync.
  Failed(String),
}

pub trait FavoriteServer {
  /// Pushes at most `PUSH_LIMIT` changes.
  fn push(
    &self,
    changes: &[FavoriteRecord],
  ) -> impl Future<Output = Result<Vec<PushResult>, SyncError>> + Send;

  fn pull(&self, cursor: Option<&str>) -> impl Future<Output = Result<PullPage, SyncError>> + Send;
}

/// What a sync did that the person must know about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
  /// Adds the server rejected. They are reverted on this device.
  pub rejected: Vec<(String, RejectReason)>,
}

/// Runs one sync on `store`. On an error, the changes applied before it stay, and records
/// that are not confirmed stay pending.
pub async fn sync(
  store: &mut FavoriteStore,
  server: &impl FavoriteServer,
) -> Result<SyncReport, SyncError> {
  let mut report = SyncReport::default();

  push(store, server, &mut report).await?;

  loop {
    let page = server.pull(store.cursor.as_deref()).await?;
    for record in page.records {
      store.apply(record);
    }
    let advanced = page.cursor.is_some() && page.cursor != store.cursor;
    if page.cursor.is_some() {
      store.cursor = page.cursor;
    }
    // Stop when the server does not move the cursor, so a faulty page cannot loop forever.
    if !page.has_more || !advanced {
      break;
    }
  }

  store.merging = false;
  Ok(report)
}

/// Pushes the pending changes in batches. During the first sync with an account, a local
/// favorite that a newer account removal beats is pushed once more with a later time, so it
/// stays.
async fn push(
  store: &mut FavoriteStore,
  server: &impl FavoriteServer,
  report: &mut SyncReport,
) -> Result<(), SyncError> {
  let mut merging = store.merging;
  let mut changes = store.pending();
  while !changes.is_empty() {
    let mut again = Vec::new();
    for batch in changes.chunks(PUSH_LIMIT) {
      for result in server.push(batch).await? {
        match result {
          PushResult::Applied(record) => {
            let pushed = batch
              .iter()
              .find(|change| change.skill_id == record.skill_id);
            let beaten_add = pushed.is_some_and(|change| change.favorited && !record.favorited);
            if merging && beaten_add {
              again.extend(store.outdate(&record.skill_id, &record));
            } else {
              store.apply(record);
            }
          },
          PushResult::Rejected {
            skill_id,
            reason,
            record,
          } => {
            if let Some(change) = batch.iter().find(|change| change.skill_id == skill_id) {
              store.revert(change);
            }
            if let Some(record) = record {
              store.apply(record);
            }
            report.rejected.push((skill_id, reason));
          },
        }
      }
    }
    // A second removal that beats the new time is a newer change; it wins.
    merging = false;
    changes = again;
  }
  Ok(())
}

/// Syncs the favorite store file with the signed-in account, one sync at a time, so a
/// sign-out cannot clear the store while a sync runs.
pub struct FavoriteSync {
  path: PathBuf,
  running: tokio::sync::Mutex<()>,
}

impl FavoriteSync {
  pub fn new(path: PathBuf) -> Self {
    Self {
      path,
      running: tokio::sync::Mutex::new(()),
    }
  }

  /// Runs one sync. `None` when no account is signed in. Local changes made while it runs
  /// stay pending for the next sync.
  pub async fn run<S, K>(
    &self,
    account: &AccountManager<S, K>,
    server: &impl FavoriteServer,
  ) -> Result<Option<SyncReport>, SyncError>
  where
    S: AccountServer,
    K: SecretStore,
  {
    let _running = self.running.lock().await;
    self.run_locked(account, server).await
  }

  /// Pushes the pending changes within `limit` (best effort), then signs out, which clears
  /// the favorites of this device.
  pub async fn sign_out<S, K>(
    &self,
    account: &AccountManager<S, K>,
    server: &impl FavoriteServer,
    limit: Duration,
  ) -> Result<(), String>
  where
    S: AccountServer,
    K: SecretStore,
  {
    let _running = self.running.lock().await;
    match tokio::time::timeout(limit, self.run_locked(account, server)).await {
      Ok(Ok(_)) => {},
      Ok(Err(error)) => tracing::warn!("Favorite sync before sign-out failed: {error:?}"),
      Err(_) => tracing::warn!("Favorite sync before sign-out timed out"),
    }
    account.sign_out().await
  }

  async fn run_locked<S, K>(
    &self,
    account: &AccountManager<S, K>,
    server: &impl FavoriteServer,
  ) -> Result<Option<SyncReport>, SyncError>
  where
    S: AccountServer,
    K: SecretStore,
  {
    let Some(identity) = account.status().await.identity else {
      return Ok(None);
    };
    let start = favorites_service::read_store(&self.path).map_err(SyncError::Failed)?;
    let mut store = start.clone();
    store.begin_account(&identity.account_id);
    let result = sync(&mut store, server).await;
    favorites_service::save_synced(&self.path, &start, store).map_err(SyncError::Failed)?;
    result.map(Some)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::FavoriteSnapshot;
  use std::sync::Mutex;

  const LIMIT: usize = 3;

  /// In-memory server with last write wins, tombstones, a change sequence and a limit.
  #[derive(Default)]
  struct FakeServer {
    state: Mutex<ServerState>,
  }

  #[derive(Default)]
  struct ServerState {
    /// Records with their change sequence.
    records: Vec<(u64, FavoriteRecord)>,
    sequence: u64,
    offline: bool,
    unauthorized: bool,
    page_size: usize,
    pushes: Vec<Vec<FavoriteRecord>>,
    pulls: Vec<Option<String>>,
  }

  impl FakeServer {
    fn new() -> Self {
      let server = Self::default();
      server.state.lock().unwrap().page_size = 100;
      server
    }

    fn with<T>(&self, f: impl FnOnce(&mut ServerState) -> T) -> T {
      f(&mut self.state.lock().unwrap())
    }

    /// A change from another device.
    fn write(&self, record: FavoriteRecord) {
      self.with(|state| state.store(record));
    }

    fn get(&self, skill_id: &str) -> Option<FavoriteRecord> {
      self.with(|state| {
        state
          .records
          .iter()
          .find(|(_, record)| record.skill_id == skill_id)
          .map(|(_, record)| record.clone())
      })
    }
  }

  impl ServerState {
    fn check(&self) -> Result<(), SyncError> {
      if self.unauthorized {
        return Err(SyncError::Unauthorized);
      }
      if self.offline {
        return Err(SyncError::Failed("offline".to_string()));
      }
      Ok(())
    }

    fn store(&mut self, record: FavoriteRecord) -> FavoriteRecord {
      let index = self
        .records
        .iter()
        .position(|(_, stored)| stored.skill_id == record.skill_id);
      if let Some(index) = index {
        // Ties keep the stored record.
        if self.records[index].1.updated_at >= record.updated_at {
          return self.records[index].1.clone();
        }
        self.records.remove(index);
      }
      self.sequence += 1;
      self.records.push((self.sequence, record.clone()));
      record
    }

    fn favorited(&self) -> usize {
      self
        .records
        .iter()
        .filter(|(_, record)| record.favorited)
        .count()
    }
  }

  impl FavoriteServer for FakeServer {
    async fn push(&self, changes: &[FavoriteRecord]) -> Result<Vec<PushResult>, SyncError> {
      self.with(|state| {
        state.check()?;
        state.pushes.push(changes.to_vec());
        Ok(
          changes
            .iter()
            .map(|change| {
              let is_new_add = change.favorited
                && !state
                  .records
                  .iter()
                  .any(|(_, record)| record.skill_id == change.skill_id && record.favorited);
              if is_new_add && state.favorited() >= LIMIT {
                return PushResult::Rejected {
                  skill_id: change.skill_id.clone(),
                  reason: RejectReason::LimitReached,
                  record: None,
                };
              }
              PushResult::Applied(state.store(change.clone()))
            })
            .collect(),
        )
      })
    }

    async fn pull(&self, cursor: Option<&str>) -> Result<PullPage, SyncError> {
      self.with(|state| {
        state.check()?;
        state.pulls.push(cursor.map(str::to_string));
        let after: u64 = cursor.map(|value| value.parse().unwrap()).unwrap_or(0);
        let mut later: Vec<_> = state
          .records
          .iter()
          .filter(|(sequence, _)| *sequence > after)
          .cloned()
          .collect();
        later.sort_by_key(|(sequence, _)| *sequence);
        let has_more = later.len() > state.page_size;
        later.truncate(state.page_size);
        let cursor = later
          .last()
          .map(|(sequence, _)| sequence.to_string())
          .or(cursor.map(str::to_string));
        Ok(PullPage {
          records: later.into_iter().map(|(_, record)| record).collect(),
          cursor,
          has_more,
        })
      })
    }
  }

  fn record(skill_id: &str, favorited: bool, updated_at: i64) -> FavoriteRecord {
    FavoriteRecord {
      skill_id: skill_id.to_string(),
      favorited,
      updated_at,
      snapshot: FavoriteSnapshot {
        name: format!("skill-{skill_id}"),
        source: "team/tools".to_string(),
        description: None,
      },
    }
  }

  fn add(store: &mut FavoriteStore, skill_id: &str, now: i64) {
    let snapshot = record(skill_id, true, now).snapshot;
    store.set(skill_id, true, Some(snapshot), now).unwrap();
  }

  fn run(store: &mut FavoriteStore, server: &FakeServer) -> Result<SyncReport, SyncError> {
    tokio::runtime::Builder::new_current_thread()
      .build()
      .unwrap()
      .block_on(sync(store, server))
  }

  fn signed_in() -> FavoriteStore {
    let mut store = FavoriteStore::default();
    store.begin_account("account-1");
    store
  }

  fn ids(store: &FavoriteStore) -> Vec<String> {
    store
      .list()
      .into_iter()
      .map(|item| item.record.skill_id)
      .collect()
  }

  #[test]
  fn a_local_change_pushes_and_is_no_longer_pending() {
    let server = FakeServer::new();
    let mut store = signed_in();
    add(&mut store, "1", 10);
    run(&mut store, &server).unwrap();
    assert_eq!(server.get("1"), Some(record("1", true, 10)));
    assert!(store.pending().is_empty());
    assert_eq!(ids(&store), ["1"]);
  }

  #[test]
  fn a_later_server_write_wins_over_an_earlier_local_change() {
    let server = FakeServer::new();
    let mut store = signed_in();
    run(&mut store, &server).unwrap();
    server.write(record("1", false, 20));
    add(&mut store, "1", 10);
    run(&mut store, &server).unwrap();
    assert!(ids(&store).is_empty());
    assert!(store.find("1").is_none());
    assert_eq!(server.get("1"), Some(record("1", false, 20)));
  }

  #[test]
  fn a_later_local_change_wins_over_an_earlier_server_write() {
    let server = FakeServer::new();
    server.write(record("1", true, 10));
    let mut store = signed_in();
    run(&mut store, &server).unwrap();
    store.set("1", false, None, 20).unwrap();
    run(&mut store, &server).unwrap();
    assert_eq!(server.get("1"), Some(record("1", false, 20)));
    assert!(ids(&store).is_empty());
  }

  #[test]
  fn a_removal_on_another_device_removes_the_favorite_here() {
    let server = FakeServer::new();
    let mut store = signed_in();
    add(&mut store, "1", 10);
    run(&mut store, &server).unwrap();
    server.write(record("1", false, 30));
    run(&mut store, &server).unwrap();
    assert!(ids(&store).is_empty());
    assert!(store.find("1").is_none());
  }

  #[test]
  fn an_add_on_another_device_appears_here() {
    let server = FakeServer::new();
    let mut store = signed_in();
    run(&mut store, &server).unwrap();
    server.write(record("2", true, 5));
    run(&mut store, &server).unwrap();
    assert_eq!(ids(&store), ["2"]);
    assert!(!store.find("2").unwrap().pending);
  }

  #[test]
  fn the_pull_starts_after_the_stored_cursor_and_reads_all_pages() {
    let server = FakeServer::new();
    server.with(|state| state.page_size = 2);
    for id in ["1", "2", "3"] {
      server.write(record(id, true, 1));
    }
    let mut store = signed_in();
    run(&mut store, &server).unwrap();
    assert_eq!(store.list().len(), 3);
    assert_eq!(store.cursor.as_deref(), Some("3"));

    server.write(record("4", false, 1));
    run(&mut store, &server).unwrap();
    let pulls = server.with(|state| state.pulls.clone());
    assert_eq!(pulls, [None, Some("2".to_string()), Some("3".to_string())]);
    assert_eq!(store.cursor.as_deref(), Some("4"));
  }

  #[test]
  fn first_sign_in_merges_local_favorites_into_the_account_as_a_union() {
    let server = FakeServer::new();
    server.write(record("remote", true, 5));
    let mut store = FavoriteStore::default();
    add(&mut store, "local", 10);
    store.cursor = Some("stale".to_string());

    store.begin_account("account-1");
    run(&mut store, &server).unwrap();

    let mut list = ids(&store);
    list.sort();
    assert_eq!(list, ["local", "remote"]);
    assert!(server.get("local").unwrap().favorited);
    assert_eq!(server.with(|state| state.pulls[0].clone()), None);
  }

  #[test]
  fn first_sign_in_keeps_a_local_favorite_over_a_newer_account_tombstone() {
    let server = FakeServer::new();
    server.write(record("1", false, 20));
    let mut store = FavoriteStore::default();
    add(&mut store, "1", 10);

    store.begin_account("account-1");
    run(&mut store, &server).unwrap();

    assert_eq!(ids(&store), ["1"]);
    assert!(store.pending().is_empty());
    let stored = server.get("1").unwrap();
    assert!(stored.favorited);
    assert!(stored.updated_at > 20);
    assert_eq!(store.find("1").unwrap().record, stored);

    // After the first sync, a later removal on another device wins again.
    server.write(record("1", false, stored.updated_at + 1));
    run(&mut store, &server).unwrap();
    assert!(ids(&store).is_empty());
  }

  #[test]
  fn pushes_send_at_most_the_push_limit_per_request() {
    let server = FakeServer::new();
    server.with(|state| state.page_size = 2_000);
    let mut store = signed_in();
    for id in 0..PUSH_LIMIT + 1 {
      add(&mut store, &id.to_string(), 10);
    }
    // The fake limit is 3 favorites; removals are not limited.
    for id in 0..PUSH_LIMIT + 1 {
      store.set(&id.to_string(), false, None, 20).unwrap();
    }
    run(&mut store, &server).unwrap();
    let sizes: Vec<_> = server.with(|state| state.pushes.iter().map(Vec::len).collect());
    assert_eq!(sizes, [PUSH_LIMIT, 1]);
    assert!(store.pending().is_empty());
  }

  #[test]
  fn a_rejected_change_takes_the_record_the_server_keeps() {
    struct Rejecting;
    impl FavoriteServer for Rejecting {
      async fn push(&self, changes: &[FavoriteRecord]) -> Result<Vec<PushResult>, SyncError> {
        Ok(
          changes
            .iter()
            .map(|change| PushResult::Rejected {
              skill_id: change.skill_id.clone(),
              reason: RejectReason::Other("invalid".to_string()),
              record: Some(record(&change.skill_id, true, 5)),
            })
            .collect(),
        )
      }
      async fn pull(&self, cursor: Option<&str>) -> Result<PullPage, SyncError> {
        Ok(PullPage {
          records: Vec::new(),
          cursor: cursor.map(str::to_string),
          has_more: false,
        })
      }
    }
    let mut store = signed_in();
    add(&mut store, "1", 10);
    let report = tokio::runtime::Builder::new_current_thread()
      .build()
      .unwrap()
      .block_on(sync(&mut store, &Rejecting))
      .unwrap();
    assert_eq!(
      report.rejected,
      [("1".to_string(), RejectReason::Other("invalid".to_string()))]
    );
    assert_eq!(store.find("1").unwrap().record, record("1", true, 5));
    assert!(store.pending().is_empty());
  }

  #[test]
  fn sign_in_to_another_account_pushes_all_records_again() {
    let first = FakeServer::new();
    let mut store = signed_in();
    add(&mut store, "1", 10);
    run(&mut store, &first).unwrap();

    let second = FakeServer::new();
    store.begin_account("account-2");
    assert_eq!(store.pending().len(), 1);
    run(&mut store, &second).unwrap();
    assert!(second.get("1").unwrap().favorited);
  }

  #[test]
  fn offline_changes_stay_pending_and_push_when_the_network_is_back() {
    let server = FakeServer::new();
    server.with(|state| state.offline = true);
    let mut store = signed_in();
    add(&mut store, "1", 10);
    assert!(matches!(
      run(&mut store, &server),
      Err(SyncError::Failed(_))
    ));
    assert_eq!(store.pending().len(), 1);
    assert_eq!(ids(&store), ["1"]);

    server.with(|state| state.offline = false);
    run(&mut store, &server).unwrap();
    assert!(store.pending().is_empty());
    assert!(server.get("1").unwrap().favorited);
  }

  #[test]
  fn an_add_above_the_limit_is_reverted_and_reported() {
    let server = FakeServer::new();
    for id in ["1", "2", "3"] {
      server.write(record(id, true, 1));
    }
    let mut store = signed_in();
    run(&mut store, &server).unwrap();
    add(&mut store, "4", 10);
    store.set("1", false, None, 10).unwrap();
    add(&mut store, "5", 10);

    let report = run(&mut store, &server).unwrap();

    // The removal frees one place, so only the second add is rejected.
    assert_eq!(
      report.rejected,
      [("5".to_string(), RejectReason::LimitReached)]
    );
    assert!(store.find("5").is_none());
    let mut list = ids(&store);
    list.sort();
    assert_eq!(list, ["2", "3", "4"]);
    assert!(store.pending().is_empty());
  }

  #[test]
  fn unauthorized_keeps_local_records_for_the_next_sign_in() {
    let server = FakeServer::new();
    let mut store = signed_in();
    add(&mut store, "1", 10);
    server.with(|state| state.unauthorized = true);
    assert_eq!(run(&mut store, &server), Err(SyncError::Unauthorized));
    assert_eq!(store.pending().len(), 1);
    assert_eq!(store.account_id.as_deref(), Some("account-1"));

    // The same account signs in again: only the pending change pushes.
    server.with(|state| state.unauthorized = false);
    store.begin_account("account-1");
    run(&mut store, &server).unwrap();
    assert_eq!(
      server.with(|state| state.pushes.clone()),
      [vec![record("1", true, 10)]]
    );
  }

  #[test]
  fn a_local_change_after_the_push_is_not_lost_by_the_pull() {
    let mut store = signed_in();
    add(&mut store, "1", 10);
    store.cursor = Some("1".to_string());
    // The server sends an older state while a newer local change waits.
    store.apply(record("1", false, 5));
    assert!(store.find("1").unwrap().pending);
    assert_eq!(ids(&store), ["1"]);
  }

  mod store_sync {
    use super::*;
    use crate::services::favorite_api_service::tests::signed_in as account;
    use std::sync::atomic::Ordering;

    fn block_on<F: Future>(future: F) -> F::Output {
      tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
    }

    #[test]
    fn a_sync_binds_the_store_to_the_account_and_saves_the_result() {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("favorites.json");
      favorites_service::update_store_for_tests(&path, |store| {
        add(store, "1", 10);
      });
      let (account, _, _) = account(path.clone());
      let server = FakeServer::new();
      server.write(record("2", true, 5));

      let report = block_on(FavoriteSync::new(path.clone()).run(&account, &server)).unwrap();

      assert_eq!(report, Some(SyncReport::default()));
      assert!(server.get("1").unwrap().favorited);
      let saved = favorites_service::load_store(&path).unwrap();
      assert_eq!(saved.account_id.as_deref(), Some("12"));
      assert!(saved.pending().is_empty());
      assert_eq!(saved.list().len(), 2);
    }

    #[test]
    fn no_sync_runs_while_signed_out() {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("favorites.json");
      let (account, _, _) = account(path.clone());
      block_on(account.sign_out()).unwrap();
      let server = FakeServer::new();
      let report = block_on(FavoriteSync::new(path).run(&account, &server)).unwrap();
      assert_eq!(report, None);
      assert!(server.with(|state| state.pulls.is_empty()));
    }

    #[test]
    fn a_favorite_added_just_before_sign_out_is_on_the_server_after_it() {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("favorites.json");
      let (account, sessions, _) = account(path.clone());
      let server = FakeServer::new();
      let sync = FavoriteSync::new(path.clone());
      block_on(sync.run(&account, &server)).unwrap();
      favorites_service::update_store_for_tests(&path, |store| {
        add(store, "1", 10);
      });

      block_on(sync.sign_out(&account, &server, Duration::from_secs(5))).unwrap();

      assert!(server.get("1").unwrap().favorited);
      assert!(sessions.signed_out.load(Ordering::SeqCst));
      assert_eq!(block_on(account.status()).identity, None);
      assert_eq!(
        favorites_service::load_store(&path).unwrap(),
        FavoriteStore::default()
      );
    }

    #[test]
    fn sign_out_finishes_when_the_server_is_unreachable() {
      let dir = tempfile::tempdir().unwrap();
      let path = dir.path().join("favorites.json");
      let (account, _, _) = account(path.clone());
      let server = FakeServer::new();
      server.with(|state| state.offline = true);
      favorites_service::update_store_for_tests(&path, |store| {
        add(store, "1", 10);
      });
      block_on(FavoriteSync::new(path.clone()).sign_out(&account, &server, Duration::from_secs(5)))
        .unwrap();
      assert_eq!(block_on(account.status()).identity, None);
      assert!(!path.exists());
    }
  }
}
