//! Local favorites: marketplace skills a person marks to revisit. They work without an
//! account and are stored in the app data directory. Sync with an account is in
//! `favorite_sync_service`.

use crate::models::{FavoriteSnapshot, FavoriteStore, LocalFavorite};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serializes read-modify-write of the store file between commands.
static STORE_LOCK: Mutex<()> = Mutex::new(());

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

/// Deletes all favorite records, the account binding and the sync cursor of this device.
pub(crate) fn clear_store(path: &Path) -> Result<(), String> {
  let _guard = lock()?;
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

fn lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
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
  fn an_invalid_file_is_an_error_and_not_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("favorites.json");
    fs::write(&path, "not json").unwrap();
    assert!(load_store(&path).is_err());
  }
}
