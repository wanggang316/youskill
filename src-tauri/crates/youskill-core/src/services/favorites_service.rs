//! Local favorites: marketplace skills a person marks to revisit. They work without an
//! account and are stored in the app data directory. Sync with an account is in
//! `favorite_sync_service`.

use crate::models::{FavoriteSnapshot, FavoriteStore, LocalFavorite};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// Serializes read-modify-write of the store file between commands. It also counts the clears
/// of each store file, so a sync can see that the store was cleared while it ran.
static STORE_LOCK: Mutex<BTreeMap<PathBuf, u64>> = Mutex::new(BTreeMap::new());

/// The store as a sync read it.
pub(crate) struct SyncStart {
  pub store: FavoriteStore,
  clears: u64,
}

pub fn list_favorites() -> Result<Vec<LocalFavorite>, String> {
  let _guard = lock()?;
  Ok(load_store(&store_path()?)?.list())
}

/// Adds or removes a favorite and returns the updated list.
pub fn set_favorite(
  skill_id: &str,
  favorited: bool,
  snapshot: Option<FavoriteSnapshot>,
) -> Result<Vec<LocalFavorite>, String> {
  let _guard = lock()?;
  update_store(&store_path()?, |store| {
    store.set(
      skill_id,
      favorited,
      snapshot,
      chrono::Utc::now().timestamp_millis(),
    )
  })
}

/// Reads the store, for a sync that runs without the lock.
pub(crate) fn read_store(path: &Path) -> Result<SyncStart, String> {
  let guard = lock()?;
  Ok(SyncStart {
    store: load_store(path)?,
    clears: clears(&guard, path),
  })
}

/// Saves the result of a sync that started from `start`. Local changes made while the sync
/// ran stay. Nothing is saved when the store was cleared (sign-out, account deletion) or
/// bound to another account while the sync ran: the result belongs to the old account.
pub(crate) fn save_synced(
  path: &Path,
  start: &SyncStart,
  mut synced: FavoriteStore,
) -> Result<(), String> {
  let guard = lock()?;
  if clears(&guard, path) != start.clears {
    return Ok(());
  }
  let current = load_store(path)?;
  if current.account_id.is_some() && current.account_id != start.store.account_id {
    return Ok(());
  }
  synced.keep_local_changes(&start.store, &current);
  if synced == current {
    return Ok(());
  }
  save_store(path, &synced)
}

/// Deletes all favorite records, the account binding and the sync cursor of this device.
pub(crate) fn clear_store(path: &Path) -> Result<(), String> {
  let mut guard = lock()?;
  *guard.entry(path.to_path_buf()).or_default() += 1;
  if path.exists() {
    fs::remove_file(path).map_err(|e| e.to_string())?;
  }
  Ok(())
}

/// Path of the favorite store in the app config directory.
pub fn store_path() -> Result<PathBuf, String> {
  let config_dir = dirs_next::config_dir().ok_or("Unable to get config directory")?;
  Ok(config_dir.join("youskill").join("favorites.json"))
}

fn clears(guard: &BTreeMap<PathBuf, u64>, path: &Path) -> u64 {
  guard.get(path).copied().unwrap_or(0)
}

fn lock() -> Result<MutexGuard<'static, BTreeMap<PathBuf, u64>>, String> {
  STORE_LOCK
    .lock()
    .map_err(|_| "Favorite store lock is poisoned".to_string())
}

fn update_store(
  path: &Path,
  change: impl FnOnce(&mut FavoriteStore) -> Result<bool, String>,
) -> Result<Vec<LocalFavorite>, String> {
  let mut store = load_store(path)?;
  if change(&mut store)? {
    save_store(path, &store)?;
  }
  Ok(store.list())
}

pub(crate) fn load_store(path: &Path) -> Result<FavoriteStore, String> {
  if !path.exists() {
    return Ok(FavoriteStore::default());
  }
  let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
  serde_json::from_str(&content).map_err(|e| format!("Invalid favorites file: {e}"))
}

pub(crate) fn save_store(path: &Path, store: &FavoriteStore) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  let content = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
  // Write then rename, so a crash cannot leave a half-written file.
  let temp = path.with_extension("json.tmp");
  fs::write(&temp, content).map_err(|e| e.to_string())?;
  fs::rename(&temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) fn update_store_for_tests(path: &Path, change: impl FnOnce(&mut FavoriteStore)) {
  let mut store = load_store(path).unwrap();
  change(&mut store);
  save_store(path, &store).unwrap();
}

#[cfg(test)]
mod tests {
  use super::*;

  fn snapshot(name: &str) -> FavoriteSnapshot {
    FavoriteSnapshot {
      name: name.to_string(),
      source: "team/tools".to_string(),
      description: Some("Reviews code".to_string()),
    }
  }

  #[test]
  fn favorites_stay_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    update_store(&path, |store| {
      store.set("42", true, Some(snapshot("review")), 1_000)
    })
    .unwrap();

    let reloaded = load_store(&path).unwrap();
    let list = reloaded.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].record.skill_id, "42");
    assert_eq!(list[0].record.updated_at, 1_000);
    assert_eq!(list[0].record.snapshot, snapshot("review"));
    assert!(list[0].pending);
  }

  #[test]
  fn a_removal_keeps_a_pending_tombstone_out_of_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    update_store(&path, |store| {
      store.set("42", true, Some(snapshot("review")), 1_000)
    })
    .unwrap();
    let list = update_store(&path, |store| store.set("42", false, None, 2_000)).unwrap();
    assert!(list.is_empty());

    let stored = load_store(&path).unwrap();
    let tombstone = stored.find("42").unwrap();
    assert!(!tombstone.record.favorited);
    assert!(tombstone.pending);
    assert_eq!(tombstone.record.updated_at, 2_000);
    assert_eq!(tombstone.record.snapshot.name, "review");
  }

  #[test]
  fn an_add_needs_a_snapshot_and_repeats_are_no_ops() {
    let mut store = FavoriteStore::default();
    assert!(store.set("42", true, None, 1).is_err());
    assert!(!store.set("42", false, None, 1).unwrap());
    assert!(store.set("42", true, Some(snapshot("review")), 1).unwrap());
    assert!(!store.set("42", true, Some(snapshot("review")), 2).unwrap());
    assert_eq!(store.find("42").unwrap().record.updated_at, 1);
  }

  #[test]
  fn a_later_change_is_later_even_when_the_clock_goes_back() {
    let mut store = FavoriteStore::default();
    store
      .set("42", true, Some(snapshot("review")), 5_000)
      .unwrap();
    store.set("42", false, None, 4_000).unwrap();
    assert_eq!(store.find("42").unwrap().record.updated_at, 5_001);
  }

  #[test]
  fn the_list_shows_the_newest_favorite_first() {
    let mut store = FavoriteStore::default();
    store.set("1", true, Some(snapshot("one")), 1).unwrap();
    store.set("2", true, Some(snapshot("two")), 3).unwrap();
    store.set("3", true, Some(snapshot("three")), 2).unwrap();
    let ids: Vec<_> = store
      .list()
      .into_iter()
      .map(|item| item.record.skill_id)
      .collect();
    assert_eq!(ids, ["2", "3", "1"]);
  }

  #[test]
  fn a_local_change_during_a_sync_stays_after_the_sync_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    update_store(&path, |store| {
      store.set("1", true, Some(snapshot("one")), 1)
    })
    .unwrap();
    let start = read_store(&path).unwrap();
    let mut synced = start.store.clone();
    synced.begin_account("12");
    synced.favorites[0].pending = false;
    synced.cursor = Some("4".to_string());

    // While the sync runs, the person removes "1" and adds "2".
    update_store(&path, |store| store.set("1", false, None, 2)).unwrap();
    update_store(&path, |store| {
      store.set("2", true, Some(snapshot("two")), 3)
    })
    .unwrap();
    save_synced(&path, &start, synced).unwrap();

    let saved = load_store(&path).unwrap();
    assert_eq!(saved.cursor.as_deref(), Some("4"));
    assert_eq!(saved.account_id.as_deref(), Some("12"));
    let one = saved.find("1").unwrap();
    assert!(!one.record.favorited && one.pending);
    assert!(saved.find("2").unwrap().pending);
  }

  #[test]
  fn a_clear_during_a_sync_stays_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    update_store(&path, |store| {
      store.set("1", true, Some(snapshot("one")), 1)
    })
    .unwrap();
    let start = read_store(&path).unwrap();
    let mut synced = start.store.clone();
    synced.begin_account("12");
    synced.cursor = Some("4".to_string());

    clear_store(&path).unwrap();
    save_synced(&path, &start, synced).unwrap();

    assert!(!path.exists());
  }

  #[test]
  fn a_sync_result_of_another_account_is_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    update_store(&path, |store| {
      store.begin_account("12");
      Ok(true)
    })
    .unwrap();
    let start = read_store(&path).unwrap();
    let mut synced = start.store.clone();
    synced.cursor = Some("4".to_string());

    update_store(&path, |store| {
      store.begin_account("13");
      Ok(true)
    })
    .unwrap();
    save_synced(&path, &start, synced).unwrap();

    let saved = load_store(&path).unwrap();
    assert_eq!(saved.account_id.as_deref(), Some("13"));
    assert_eq!(saved.cursor, None);
  }

  #[test]
  fn an_invalid_file_is_an_error_and_not_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    fs::write(&path, "not json").unwrap();
    assert!(load_store(&path).is_err());
  }
}
