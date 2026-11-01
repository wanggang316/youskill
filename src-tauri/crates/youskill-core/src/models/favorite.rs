use serde::{Deserialize, Serialize};

/// What a favorite shows when its skill is no longer in the marketplace. Taken when the
/// skill is favorited.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteSnapshot {
  pub name: String,
  pub source: String,
  #[serde(default)]
  pub description: Option<String>,
}

/// One favorite record, identified by the marketplace skill id. A record with `favorited`
/// false is a removal (tombstone) that sync must still send or apply.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteRecord {
  pub skill_id: String,
  pub favorited: bool,
  /// Client time of the change in milliseconds since the Unix epoch. Last write wins.
  pub updated_at: i64,
  pub snapshot: FavoriteSnapshot,
}

/// A favorite record kept on this device.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFavorite {
  #[serde(flatten)]
  pub record: FavoriteRecord,
  /// The change is not sent to the account yet.
  #[serde(default)]
  pub pending: bool,
}

/// All favorite data of this device, stored in the app data directory.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct FavoriteStore {
  /// Account the records were last synced with. `None` before the first sign-in.
  pub account_id: Option<String>,
  /// Server change sequence of the last pull.
  pub cursor: Option<String>,
  pub favorites: Vec<LocalFavorite>,
}

impl FavoriteStore {
  /// Favorited records, newest change first.
  pub fn list(&self) -> Vec<LocalFavorite> {
    let mut out: Vec<LocalFavorite> = self
      .favorites
      .iter()
      .filter(|item| item.record.favorited)
      .cloned()
      .collect();
    out.sort_by_key(|item| std::cmp::Reverse(item.record.updated_at));
    out
  }

  pub fn find(&self, skill_id: &str) -> Option<&LocalFavorite> {
    self
      .favorites
      .iter()
      .find(|item| item.record.skill_id == skill_id)
  }

  /// Records a local add or removal as a pending change. An add needs a snapshot; a removal
  /// keeps the saved one. Returns whether the state changed.
  pub fn set(
    &mut self,
    skill_id: &str,
    favorited: bool,
    snapshot: Option<FavoriteSnapshot>,
    now: i64,
  ) -> Result<bool, String> {
    let index = self
      .favorites
      .iter()
      .position(|item| item.record.skill_id == skill_id);
    if let Some(index) = index {
      let item = &mut self.favorites[index];
      if item.record.favorited == favorited {
        return Ok(false);
      }
      item.record.favorited = favorited;
      // A clock that went back must not lose against the previous change.
      item.record.updated_at = now.max(item.record.updated_at + 1);
      if let Some(snapshot) = snapshot.filter(|_| favorited) {
        item.record.snapshot = snapshot;
      }
      item.pending = true;
      return Ok(true);
    }
    if !favorited {
      return Ok(false);
    }
    let snapshot = snapshot.ok_or("A favorite needs the skill name and source")?;
    self.favorites.push(LocalFavorite {
      record: FavoriteRecord {
        skill_id: skill_id.to_string(),
        favorited,
        updated_at: now,
        snapshot,
      },
      pending: true,
    });
    Ok(true)
  }

  pub fn pending(&self) -> Vec<FavoriteRecord> {
    self
      .favorites
      .iter()
      .filter(|item| item.pending)
      .map(|item| item.record.clone())
      .collect()
  }

  /// Applies a record from the server by last write wins. A later local change that is not
  /// sent yet stays. A removal that the server confirms leaves no local record.
  pub fn apply(&mut self, record: FavoriteRecord) {
    let index = self
      .favorites
      .iter()
      .position(|item| item.record.skill_id == record.skill_id);
    match index {
      Some(index) => {
        let local = &self.favorites[index];
        if local.pending && local.record.updated_at > record.updated_at {
          return;
        }
        if record.favorited {
          self.favorites[index] = LocalFavorite {
            record,
            pending: false,
          };
        } else {
          self.favorites.remove(index);
        }
      },
      None if record.favorited => self.favorites.push(LocalFavorite {
        record,
        pending: false,
      }),
      None => {},
    }
  }

  /// Reverts a pushed add that the server rejected. A newer local change stays.
  pub fn revert(&mut self, rejected: &FavoriteRecord) {
    self.favorites.retain(|item| {
      item.record.skill_id != rejected.skill_id || item.record.updated_at != rejected.updated_at
    });
  }

  /// Prepares the records for a signed-in account. For the same account the pending changes
  /// push as usual. For a new account all records push and the pull starts from no cursor,
  /// so local favorites merge into the account as a union.
  pub fn begin_account(&mut self, account_id: &str) {
    if self.account_id.as_deref() == Some(account_id) {
      return;
    }
    self.account_id = Some(account_id.to_string());
    self.cursor = None;
    for item in &mut self.favorites {
      item.pending = true;
    }
  }
}
