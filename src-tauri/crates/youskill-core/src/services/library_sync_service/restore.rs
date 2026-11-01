//! Restore the library from the repository, item by item.
//!
//! Each item this device gets (a hub skill, an instruction template, a custom agent) is
//! compared in three states: the base (the last synced commit, kept in the sync state), this computer
//! (the snapshot a backup would build) and the target commit. What changed on one side only
//! is taken from that side, what changed on both is a conflict the user decides, and a
//! remote removal waits for confirmation. `plan_restore` computes the comparison;
//! `apply_restore` applies it with the user's decisions and commits the merged result on top
//! of the branch, so other computers fast-forward as usual. The device-only items and device
//! files of other devices are never written here and are kept from the head.

use super::checkpoint::{self, Link};
use super::*;
use crate::models::{new_skill_id, InstructionRecord};
use crate::services::install_service::write_target;
use crate::utils::folder::{copy_dir, CopyOpts, SKILL_MD};
use crate::utils::git_remote::{RemoteCommit, RemoteHead};
use crate::utils::path::{is_symlink, symlink_points_to};

/// How many commits the history lists, and how far back a restore target can be.
const HISTORY_LIMIT: usize = 50;
/// Suffix of the local copy that "keep both" adds as a new hub skill.
const SKILL_COPY_SUFFIX: &str = "-local";
/// Suffix of the local copy that "keep both" adds as a new instruction template.
const INSTRUCTION_COPY_SUFFIX: &str = " (local)";

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
  /// Merge against the last synced commit.
  Pull,
  /// First connect: merge both libraries; equal items are recognized as one.
  Merge,
  /// First connect: the repository wins every item; local-only items are removal
  /// candidates.
  ReplaceLocal,
  /// First connect: this computer's library replaces the repository's.
  ReplaceRemote,
}

/// What happened to an item on one side since the base.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemChange {
  Unchanged,
  Added,
  Modified,
  Removed,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
  /// Changed here only: the local version stays and goes to the repository.
  KeepLocal,
  /// Changed in the repository only: the repository version is written here, unless the
  /// user excludes it.
  TakeRemote,
  /// Changed on both sides: the user decides.
  Conflict,
  /// Removed in the repository: removed here only when the user confirms.
  RemovalCandidate,
}

/// Why a local item and a repository item with different ids are one item (first merge).
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchReason {
  /// Equal content: the same item.
  Hash,
  /// Equal source, different content: a conflict.
  SourceKey,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreItem {
  /// `skill:<id>`, `instruction:<id>` or `agent:<id>`, with the repository's id.
  pub key: String,
  pub kind: ItemKind,
  pub id: String,
  pub name: String,
  /// The local id of an item paired with a repository item of another id.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub local_id: Option<String>,
  /// The local name when it differs from `name`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub local_name: Option<String>,
  pub local_change: ItemChange,
  pub remote_change: ItemChange,
  pub resolution: Resolution,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub match_reason: Option<MatchReason>,
  /// The user left this repository version out of an earlier restore; it is excluded
  /// unless included again.
  #[serde(default)]
  pub deferred: bool,
  /// Another device made this shared item its device-only item: removed by default. A
  /// kept local version becomes a device-only item of this device with a new id.
  #[serde(default)]
  pub made_device_only: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
  Ready,
  /// Nothing to restore: the target is the last synced commit and nothing differs.
  UpToDate,
  /// The repository holds no library yet: back up instead.
  RemoteEmpty,
  /// First connect with a library on both sides: choose `merge`, `replace_local` or
  /// `replace_remote`.
  ChooseMode,
  /// The repository holds a version 1 library; migrate it first (`migrate`).
  MigrationRequired,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
  pub state: PlanState,
  pub mode: RestoreMode,
  /// The requested commit; `None` is the branch head.
  #[serde(default)]
  pub target: Option<String>,
  /// The commit whose library is restored.
  #[serde(default)]
  pub target_commit: Option<String>,
  /// The branch head the plan was computed against.
  #[serde(default)]
  pub remote_head: Option<String>,
  /// Fingerprint of this computer's library when the plan was computed.
  pub fingerprint: String,
  pub items: Vec<RestoreItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
  KeepLocal,
  TakeRemote,
  /// The repository version keeps the id; the local version is kept as a separate item.
  KeepBoth,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemovalChoice {
  Remove,
  Keep,
}

/// The user's decisions, by item key.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RestoreDecisions {
  /// Required for every conflict.
  #[serde(default)]
  pub conflicts: BTreeMap<String, ConflictChoice>,
  /// Default `keep`, except `remove` for an item another device made device-only.
  #[serde(default)]
  pub removals: BTreeMap<String, RemovalChoice>,
  /// Include (`true`) or exclude a `take_remote` item. Default: include, except a deferred
  /// item (an incoming item, which a backup kept from another device, is included).
  #[serde(default)]
  pub include: BTreeMap<String, bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOptions {
  /// After the restore, run the pending installs and copy updates for the agents of this
  /// computer. Uninstalls stay manual.
  #[serde(default)]
  pub apply_installs: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreResult {
  Applied,
  /// The repository head or this computer's library changed since the plan; plan again.
  /// Nothing was written, unless `localApplied` says the local result stays because the
  /// branch moved while the merge commit was pushed.
  Stale,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
  pub result: RestoreResult,
  /// The new last synced commit.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub commit: Option<String>,
  /// A merge commit was pushed.
  pub pushed: bool,
  /// The hub and lock files were changed.
  pub local_applied: bool,
  /// The custom agent definitions changed; agent lists need a refresh.
  pub agents_changed: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub installs: Option<ApplyPendingOutcome>,
}

impl RestoreOutcome {
  fn stale(local_applied: bool) -> Self {
    Self {
      result: RestoreResult::Stale,
      commit: None,
      pushed: false,
      local_applied,
      agents_changed: false,
      installs: None,
    }
  }
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

pub fn history(env: &Env) -> Result<Vec<RemoteCommit>, String> {
  let remote = github_remote(env)?;
  history_with(env, &remote)
}

/// Recent commits of the branch, newest first.
pub fn history_with(env: &Env, remote: &dyn RemoteRepo) -> Result<Vec<RemoteCommit>, String> {
  read_state(env)?.connected()?;
  remote.history(HISTORY_LIMIT)
}

// ---------------------------------------------------------------------------
// Plan
// ---------------------------------------------------------------------------

pub fn plan_restore(
  env: &Env,
  mode: RestoreMode,
  target: Option<&str>,
) -> Result<RestorePlan, String> {
  let remote = github_remote(env)?;
  plan_restore_with(env, &remote, mode, target)
}

pub fn plan_restore_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  mode: RestoreMode,
  target: Option<&str>,
) -> Result<RestorePlan, String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  ensure_device(env, &mut state)?;
  fill_base_agents(env, remote, &mut state)?;
  Ok(compute_plan(env, remote, &state, mode, target)?.plan)
}

/// A plan with what applying it needs.
struct Planned {
  plan: RestorePlan,
  device: String,
  head: Option<RemoteHead>,
  /// The head commit, for the parts of other devices.
  head_state: RepoState,
  target_state: RepoState,
  /// Local items, keyed by the repository key after first-merge pairing.
  local: BTreeMap<String, ItemVersion>,
  remote: BTreeMap<String, ItemVersion>,
  /// Repository key -> (local key, reason) of paired items.
  pairs: BTreeMap<String, (String, MatchReason)>,
}

pub(super) fn change(base: Option<&ItemVersion>, side: Option<&ItemVersion>) -> ItemChange {
  match (base, side) {
    (None, None) => ItemChange::Unchanged,
    (None, Some(_)) => ItemChange::Added,
    (Some(_), None) => ItemChange::Removed,
    (Some(base), Some(side)) if base.same_content(side) => ItemChange::Unchanged,
    _ => ItemChange::Modified,
  }
}

pub(super) fn resolve(
  local_change: ItemChange,
  remote_change: ItemChange,
  local: Option<&ItemVersion>,
  remote: Option<&ItemVersion>,
) -> Option<Resolution> {
  if let (Some(local), Some(remote)) = (local, remote) {
    if local.same_content(remote) {
      return None;
    }
  }
  match (local_change, remote_change) {
    (ItemChange::Unchanged, ItemChange::Unchanged) => None,
    (_, ItemChange::Unchanged) => Some(Resolution::KeepLocal),
    (ItemChange::Unchanged, ItemChange::Removed) => Some(Resolution::RemovalCandidate),
    (ItemChange::Unchanged, _) => Some(Resolution::TakeRemote),
    _ => match (local, remote) {
      (None, None) => None,
      // Both made the same change: only the hash and timestamps differ.
      (Some(local), Some(remote)) if local.same_except_times(remote) => {
        Some(Resolution::TakeRemote)
      },
      _ => Some(Resolution::Conflict),
    },
  }
}

/// Pair local and repository items of the same kind that have different ids: equal content
/// first, then equal source key. Paired local items move to the repository key. Custom
/// agents pair only by id.
pub(super) fn pair_items(
  local: &mut BTreeMap<String, ItemVersion>,
  remote: &BTreeMap<String, ItemVersion>,
) -> BTreeMap<String, (String, MatchReason)> {
  let local_only: Vec<String> = local
    .keys()
    .filter(|key| local[*key].kind() != ItemKind::Agent && !remote.contains_key(*key))
    .cloned()
    .collect();
  let mut remote_free: BTreeSet<String> = remote
    .keys()
    .filter(|key| remote[*key].kind() != ItemKind::Agent && !local.contains_key(*key))
    .cloned()
    .collect();
  let mut pairs: BTreeMap<String, (String, MatchReason)> = BTreeMap::new();
  for reason in [MatchReason::Hash, MatchReason::SourceKey] {
    for local_key in &local_only {
      if pairs.values().any(|(paired, _)| paired == local_key) {
        continue;
      }
      let version = &local[local_key];
      let found = remote_free
        .iter()
        .find(|remote_key| {
          let other = &remote[*remote_key];
          other.kind() == version.kind()
            && match reason {
              MatchReason::Hash => other.files == version.files,
              MatchReason::SourceKey => {
                version.source_key().is_some() && version.source_key() == other.source_key()
              },
            }
        })
        .cloned();
      if let Some(remote_key) = found {
        remote_free.remove(&remote_key);
        pairs.insert(remote_key, (local_key.clone(), reason));
      }
    }
  }
  for (remote_key, (local_key, _)) in &pairs {
    if let Some(version) = local.remove(local_key) {
      local.insert(remote_key.clone(), version);
    }
  }
  pairs
}

/// Identifies this computer's library at plan time, so an out-of-date plan is refused.
fn fingerprint(state: &SyncState, snapshot: &Snapshot) -> Result<String, String> {
  let value = serde_json::json!({
    "base": state.last_synced_commit,
    "deferred": state.deferred.keys().collect::<Vec<_>>(),
    "incoming": state.incoming.keys().collect::<Vec<_>>(),
    "library": snapshot.library,
    "agents": snapshot.agents,
    "device": snapshot.device,
    "tree": snapshot.item_tree(),
  });
  Ok(blob_sha(
    serde_json::to_vec(&value)
      .map_err(|e| e.to_string())?
      .as_slice(),
  ))
}

fn compute_plan(
  env: &Env,
  remote: &dyn RemoteRepo,
  state: &SyncState,
  mode: RestoreMode,
  target: Option<&str>,
) -> Result<Planned, String> {
  let device = state.device();
  // The real local library: deferred and incoming items are compared with their local
  // version.
  let snapshot = local_snapshot(
    env,
    &SyncState {
      deferred: BTreeMap::new(),
      incoming: BTreeMap::new(),
      ..state.clone()
    },
    &device,
  )?;
  let head = remote.head()?;
  let mut planned = Planned {
    plan: RestorePlan {
      state: PlanState::RemoteEmpty,
      mode,
      target: target.map(String::from),
      target_commit: None,
      remote_head: head.as_ref().map(|head| head.commit.clone()),
      fingerprint: fingerprint(state, &snapshot)?,
      items: Vec::new(),
    },
    device: device.clone(),
    head: head.clone(),
    head_state: RepoState::default(),
    target_state: RepoState::default(),
    local: BTreeMap::new(),
    remote: BTreeMap::new(),
    pairs: BTreeMap::new(),
  };

  let Some(head) = head else {
    return Ok(planned);
  };
  let head_files = remote.tree(&head.tree)?;
  let Some(head_state) = load_repo_state(remote, &head.commit, &head_files, &device)? else {
    if target.is_some() {
      return Err("That commit holds no library".to_string());
    }
    return Ok(planned);
  };
  if head_state.library.version < LIBRARY_VERSION {
    planned.plan.state = PlanState::MigrationRequired;
    return Ok(planned);
  }
  let (target_commit, target_state) = match target {
    None => (head.commit.clone(), head_state.clone()),
    Some(id) if id == head.commit => (head.commit.clone(), head_state.clone()),
    Some(id) => {
      let commit = remote
        .history(HISTORY_LIMIT)?
        .into_iter()
        .find(|commit| commit.id == id)
        .ok_or_else(|| format!("Commit {} is not in the recent history of the branch", id))?;
      let files = remote.tree(&commit.tree)?;
      let target_state = load_repo_state(remote, &commit.id, &files, &device)?
        .ok_or_else(|| "That commit holds no library".to_string())?;
      (commit.id, target_state)
    },
  };
  planned.plan.target_commit = Some(target_commit.clone());

  let first = state.last_synced_commit.is_none();
  let local_empty = snapshot.library.skills.is_empty() && snapshot.library.instructions.is_empty();
  if first && mode == RestoreMode::Pull && !local_empty {
    planned.plan.state = PlanState::ChooseMode;
    planned.head_state = head_state;
    planned.target_state = target_state;
    return Ok(planned);
  }

  let mut local = snapshot.items();
  let remote_items = target_state.items(&device);
  let pairs = if first && matches!(mode, RestoreMode::Merge | RestoreMode::ReplaceLocal) {
    pair_items(&mut local, &remote_items)
  } else {
    BTreeMap::new()
  };
  let base = match mode {
    RestoreMode::ReplaceLocal => local.clone(),
    RestoreMode::ReplaceRemote => remote_items.clone(),
    RestoreMode::Pull | RestoreMode::Merge if !first => state.base().items(&device),
    RestoreMode::Pull | RestoreMode::Merge => BTreeMap::new(),
  };
  let honor_carried = matches!(mode, RestoreMode::Pull | RestoreMode::Merge);

  let keys: BTreeSet<&String> = base
    .keys()
    .chain(local.keys())
    .chain(remote_items.keys())
    .collect();
  let mut items = Vec::new();
  for key in keys {
    let (l, r) = (local.get(key), remote_items.get(key));
    let deferred_version = state.deferred.get(key).filter(|_| honor_carried);
    let incoming = state.incoming.get(key).filter(|_| honor_carried);
    // A deferred item is compared with its local version: the repository version is
    // offered again until the user takes it. An incoming item is compared with the base
    // it had before the other device changed it.
    let b = match (incoming, deferred_version) {
      (Some(incoming), _) => incoming.base.as_ref(),
      (None, Some(_)) => l,
      (None, None) => base.get(key),
    };
    let (local_change, remote_change) = (change(b, l), change(b, r));
    let paired = pairs.get(key);
    let resolution = match paired {
      Some((_, MatchReason::Hash)) => Some(Resolution::TakeRemote),
      Some((_, MatchReason::SourceKey)) if mode == RestoreMode::Merge => Some(Resolution::Conflict),
      Some(_) => Some(Resolution::TakeRemote),
      None => resolve(local_change, remote_change, l, r),
    };
    let Some(resolution) = resolution else {
      continue;
    };
    let Some((kind, id)) = split_key(key) else {
      continue;
    };
    let name = r.or(l).or(b).map(ItemVersion::name).unwrap_or_default();
    let local_name = l.map(ItemVersion::name).filter(|local| *local != name);
    items.push(RestoreItem {
      key: key.clone(),
      kind,
      id: id.to_string(),
      name,
      local_id: paired
        .and_then(|(local_key, _)| split_key(local_key))
        .map(|(_, id)| id.to_string()),
      local_name,
      local_change,
      remote_change,
      resolution,
      match_reason: paired.map(|(_, reason)| *reason),
      deferred: incoming.is_none()
        && deferred_version
          .is_some_and(|version| r.is_some_and(|remote| remote.same_content(version))),
      made_device_only: r.is_none()
        && target_state
          .scope_of(key)
          .is_some_and(|scope| !device_gets(scope, &device)),
    });
  }
  items.sort_by(|a, b| {
    (a.kind, a.name.to_lowercase(), &a.id).cmp(&(b.kind, b.name.to_lowercase(), &b.id))
  });

  planned.plan.state =
    if items.is_empty() && state.last_synced_commit.as_deref() == Some(target_commit.as_str()) {
      PlanState::UpToDate
    } else {
      PlanState::Ready
    };
  planned.plan.items = items;
  planned.head_state = head_state;
  planned.target_state = target_state;
  planned.local = local;
  planned.remote = remote_items;
  planned.pairs = pairs;
  Ok(planned)
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// What the decisions turn into on this computer.
#[derive(Default)]
struct Actions {
  /// (local key, repository key): move a local item to the repository's id.
  rekeys: Vec<(String, String)>,
  /// (key, new id): keep the local version of `key` as a new item.
  copies: Vec<(String, String)>,
  /// Keys whose repository version is written here.
  writes: BTreeSet<String>,
  /// Local keys removed here.
  removals: Vec<String>,
  /// Repository versions the user left out.
  deferred: BTreeMap<String, ItemVersion>,
  /// (key, new id): a local item whose id is a device-only item of another device at the
  /// head moves to a new id as a device-only item of this device.
  reids: Vec<(String, String)>,
}

impl Actions {
  fn from_decisions(planned: &Planned, decisions: &RestoreDecisions) -> Result<Self, String> {
    let mut actions = Actions::default();
    for item in &planned.plan.items {
      let key = &item.key;
      let paired = planned.pairs.get(key).map(|(local_key, _)| local_key);
      let rekey = |actions: &mut Actions| {
        if let Some(local_key) = paired {
          actions.rekeys.push((local_key.clone(), key.clone()));
        }
      };
      match item.resolution {
        Resolution::KeepLocal => rekey(&mut actions),
        Resolution::TakeRemote => {
          rekey(&mut actions);
          if decisions
            .include
            .get(key)
            .copied()
            .unwrap_or(!item.deferred)
          {
            actions.writes.insert(key.clone());
          } else if let Some(version) = planned.remote.get(key) {
            actions.deferred.insert(key.clone(), version.clone());
          }
        },
        Resolution::RemovalCandidate => {
          let default = if item.made_device_only {
            RemovalChoice::Remove
          } else {
            RemovalChoice::Keep
          };
          if decisions.removals.get(key).copied().unwrap_or(default) == RemovalChoice::Remove {
            actions.removals.push(key.clone());
          }
        },
        Resolution::Conflict => {
          let choice = decisions.conflicts.get(key).ok_or_else(|| {
            format!(
              "Choose which version of {} to keep",
              if item.name.is_empty() {
                &item.key
              } else {
                &item.name
              }
            )
          })?;
          match choice {
            ConflictChoice::KeepLocal => rekey(&mut actions),
            ConflictChoice::TakeRemote => {
              rekey(&mut actions);
              if planned.remote.contains_key(key) {
                actions.writes.insert(key.clone());
              } else {
                actions.removals.push(key.clone());
              }
            },
            ConflictChoice::KeepBoth => {
              let both = planned.local.contains_key(key) && planned.remote.contains_key(key);
              if !both || item.kind == ItemKind::Agent {
                return Err(format!(
                  "{} cannot be kept twice: choose this computer's or the repository's version",
                  if item.name.is_empty() {
                    &item.key
                  } else {
                    &item.name
                  }
                ));
              }
              // A paired local item first moves to the repository id, so both cases are
              // the same: the id and its installs get the repository version, the local
              // version becomes a new item.
              rekey(&mut actions);
              actions
                .copies
                .push((key.clone(), new_item_id(item.kind, &item.id)));
              actions.writes.insert(key.clone());
            },
          }
        },
      }
    }
    // What stays here after the restore must not take the id of an item of another device.
    let foreign = planned.head_state.foreign(&planned.device).keys();
    let removed: BTreeSet<&String> = actions.removals.iter().collect();
    let staying: BTreeSet<&String> = planned
      .local
      .keys()
      .chain(actions.writes.iter())
      .filter(|key| !removed.contains(key) && foreign.contains(*key))
      .collect();
    for key in staying {
      if let Some((kind, id)) = split_key(key) {
        actions.reids.push((key.clone(), new_item_id(kind, id)));
      }
    }
    Ok(actions)
  }
}

/// A new id for a local copy of the item `id`.
fn new_item_id(kind: ItemKind, id: &str) -> String {
  match kind {
    ItemKind::Skill => new_skill_id(),
    ItemKind::Agent => super::scope::new_agent_id(id),
    _ => uuid::Uuid::new_v4().to_string(),
  }
}

fn key_of(kind: ItemKind, id: &str) -> String {
  match kind {
    ItemKind::Skill => skill_key(id),
    ItemKind::Instruction => instruction_key(id),
    _ => agent_key(id),
  }
}

pub fn apply_restore(
  env: &Env,
  plan: &RestorePlan,
  decisions: &RestoreDecisions,
  options: &RestoreOptions,
  install_mode: InstallMode,
) -> Result<RestoreOutcome, String> {
  let remote = github_remote(env)?;
  apply_restore_with(env, &remote, plan, decisions, options, install_mode)
}

/// Apply `plan` with the user's decisions. The plan is computed again and refused as
/// `stale` when the branch head or this computer's library changed since. The operation
/// lock is held from the check to the commit. Downloads and checks run before the first
/// local write; a checkpoint is taken just before it. The new base is recorded only after
/// every item was written or removed, so a failure leaves the base as it was and the
/// checkpoint to undo what was written.
pub fn apply_restore_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  plan: &RestorePlan,
  decisions: &RestoreDecisions,
  options: &RestoreOptions,
  install_mode: InstallMode,
) -> Result<RestoreOutcome, String> {
  normalize_library(env)?;
  let ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  ensure_device(env, &mut state)?;
  fill_base_agents(env, remote, &mut state)?;
  let planned = compute_plan(env, remote, &state, plan.mode, plan.target.as_deref())?;
  if planned.plan.remote_head != plan.remote_head
    || planned.plan.target_commit != plan.target_commit
    || planned.plan.fingerprint != plan.fingerprint
  {
    return Ok(RestoreOutcome::stale(false));
  }
  let (Some(head), Some(target_commit)) =
    (planned.head.clone(), planned.plan.target_commit.clone())
  else {
    return Err("The repository holds no library yet. Back up instead".to_string());
  };
  match planned.plan.state {
    PlanState::ChooseMode => {
      return Err("Choose how to combine this library with the repository".to_string())
    },
    PlanState::MigrationRequired => {
      return Err("Update the repository to the new library format first".to_string())
    },
    _ => {},
  }
  let actions = Actions::from_decisions(&planned, decisions)?;
  let prepared = prepare_remote_items(env, remote, &planned, &target_commit, &actions.writes)?;

  let (paths, links) = checkpoint_scope(env, &actions)?;
  checkpoint::take(env, &paths, &links, &target_commit)?;

  for (local_key, remote_key) in &actions.rekeys {
    rekey(env, local_key, remote_key)?;
  }
  for (key, new_id) in &actions.copies {
    let detach = match (planned.local.get(key), planned.remote.get(key)) {
      (Some(local), Some(remote)) => {
        local.source_key().is_some() && local.source_key() == remote.source_key()
      },
      _ => false,
    };
    copy_local(env, key, new_id, detach)?;
  }
  write_prepared(env, &planned, &prepared)?;
  let agents_changed = write_agents(env, &planned, &actions)?;
  // Installs stay where they are; symlinks into a removed item become real copies.
  let mut errors = Vec::new();
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  for key in &actions.removals {
    let result = match split_key(key) {
      Some((ItemKind::Skill, id)) if lock.skills.contains_key(id) => {
        hub_service::remove_hub_skill(env, id, false)
      },
      Some((ItemKind::Instruction, id)) if instruction_lock.instructions.contains_key(id) => {
        instruction_service::remove_instruction(env, id, false)
      },
      _ => Ok(()),
    };
    if let Err(err) = result {
      errors.push(err);
    }
  }
  if !errors.is_empty() {
    return Err(format!(
      "The restore stopped before it was recorded. Undo it or try again.\n{}",
      errors.join("\n")
    ));
  }
  let mut reided = Vec::new();
  for (key, new_id) in &actions.reids {
    let Some((kind, id)) = split_key(key) else {
      continue;
    };
    let new_key = key_of(kind, new_id);
    match kind {
      ItemKind::Agent => super::scope::reid_agent(env, id, new_id)?,
      _ => {
        rekey(env, key, &new_key)?;
        // The item of the other device keeps the source key.
        super::scope::detach_source(env, &new_key)?;
      },
    }
    reided.push((key.clone(), new_key));
  }

  // The base becomes the target. Its device file for this device is the base of the
  // installs here, so the pending list shows what it changes. A target without that file
  // (from before this device joined, a head before its first backup, a version 1 commit)
  // says nothing about the installs here, so they stay as they are: a restore of old
  // content must not offer to uninstall everything.
  let mut next = state.clone();
  if let Some(device) = &planned.target_state.device {
    next.base_device = Some(device.clone());
  }
  next.last_synced_commit = Some(target_commit.clone());
  next.last_pull_at = Some(now_rfc3339());
  next.base_files = planned.target_state.files.clone();
  next.base_library = Some(planned.target_state.library.clone());
  next.base_agents = Some(planned.target_state.agents.clone());
  next.deferred = actions.deferred.clone();
  next.incoming.clear();
  for (key, new_key) in &reided {
    next.scopes.remove(key);
    next
      .scopes
      .insert(new_key.clone(), Scope::Device(planned.device.clone()));
  }
  write_state(env, &next)?;

  let mut outcome = commit_result(env, remote, &planned, &head, &target_commit)?;
  outcome.agents_changed =
    agents_changed || reided.iter().any(|(key, _)| key.starts_with("agent:"));
  checkpoint::seal(env)?;
  if options.apply_installs && outcome.result == RestoreResult::Applied {
    let items: Vec<PendingItem> = pending(env)?
      .into_iter()
      .filter(|item| item.action != PendingAction::Uninstall)
      .collect();
    // The install targets go into the checkpoint first, so an undo also takes them back.
    checkpoint::add(env, &install_targets(env, &items))?;
    // The install actions take the operation lock themselves.
    drop(ops);
    outcome.installs = Some(apply_pending(env, &items, install_mode)?);
    let _ops = ops_guard(env)?;
    checkpoint::seal(env)?;
  }
  Ok(outcome)
}

/// Where the pending installs and copy updates write.
fn install_targets(env: &Env, items: &[PendingItem]) -> Vec<PathBuf> {
  items
    .iter()
    .filter_map(|item| {
      if let Some(path) = &item.path {
        return Some(PathBuf::from(path));
      }
      let spec = InstallTargetSpec {
        scope: InstallScope::User,
        project_path: None,
        agent_id: item.agent_id.clone(),
      };
      match item.kind {
        ItemKind::Skill => install_service::resolve_target(env, &item.name, &spec)
          .ok()
          .map(|target| target.path),
        ItemKind::Instruction => instruction_service::resolve_target(env, &spec)
          .ok()
          .map(|target| target.path),
        ItemKind::Agent | ItemKind::Device => None,
      }
    })
    .collect()
}

/// Commit this computer's library on top of the branch head when it differs from it, and
/// make the result the new base. The parts of other devices come from the head. Called
/// under the operation lock.
fn commit_result(
  env: &Env,
  remote: &dyn RemoteRepo,
  planned: &Planned,
  head: &RemoteHead,
  target_commit: &str,
) -> Result<RestoreOutcome, String> {
  let mut state = read_state(env)?;
  let device = state.device();
  let snapshot = local_snapshot(env, &state, &device)?;
  let foreign = planned.head_state.foreign(&device);
  let head_files = &planned.head_state.files;
  let mut composed = compose(&snapshot, &foreign, &device, None)?;
  let mut outcome = RestoreOutcome {
    result: RestoreResult::Applied,
    commit: None,
    pushed: false,
    local_applied: true,
    agents_changed: false,
    installs: None,
  };

  if &composed.tree != head_files {
    let now = now_rfc3339();
    composed = compose(&snapshot, &foreign, &device, Some(now.clone()))?;
    upload_blobs(
      remote,
      &composed.contents,
      &composed.tree,
      &[head_files, &planned.target_state.files],
    )?;
    let changes = local_changes(&planned.target_state, &snapshot, &device);
    let message = if head.commit == target_commit {
      format!(
        "Merge on {}",
        commit_message(&state.device_label(), &changes)
      )
    } else {
      format!(
        "Restore {} on {}",
        &target_commit[..target_commit.len().min(7)],
        commit_message(&state.device_label(), &changes)
      )
    };
    let tree_sha = remote.create_tree(&composed.tree)?;
    let commit = remote.create_commit(&message, &tree_sha, Some(&head.commit))?;
    match remote.update_head(&commit) {
      Ok(()) => {},
      // The local result stays with the target as base; a later backup or restore
      // reconciles it with the new head.
      Err(UpdateError::Rejected) => return Ok(RestoreOutcome::stale(true)),
      Err(UpdateError::Other(err)) => return Err(err),
    }
    state.last_backup_at = Some(now);
    outcome.pushed = true;
    outcome.commit = Some(commit.clone());
    set_base(&mut state, commit, composed, &snapshot);
  } else {
    outcome.commit = Some(head.commit.clone());
    set_base(&mut state, head.commit.clone(), composed, &snapshot);
  }
  write_state(env, &state)?;
  Ok(outcome)
}

/// Files the restore may change, and the symlink installs that point into them.
fn checkpoint_scope(env: &Env, actions: &Actions) -> Result<(Vec<PathBuf>, Vec<Link>), String> {
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  let mut paths = vec![
    env.lock_path.clone(),
    env.instruction_lock_path.clone(),
    state_path(env),
    user_agent_apps_path(env),
  ];
  let mut links = Vec::new();
  let mut keys: Vec<String> = Vec::new();
  for (local_key, remote_key) in &actions.rekeys {
    keys.push(local_key.clone());
    keys.push(remote_key.clone());
  }
  for (key, new_id) in actions.copies.iter().chain(actions.reids.iter()) {
    keys.push(key.clone());
    if let Some((kind, _)) = split_key(key) {
      keys.push(key_of(kind, new_id));
    }
  }
  keys.extend(actions.writes.iter().cloned());
  keys.extend(actions.removals.iter().cloned());
  for key in keys {
    match split_key(&key) {
      Some((ItemKind::Skill, id)) => {
        paths.push(env.hub_root.join(id));
        if let Some(record) = lock.skills.get(id) {
          let hub_dir = env.hub_dir(id, &record.name);
          links.extend(symlink_installs(
            &record.installs,
            &hub_dir,
            ItemKind::Skill,
          ));
        }
      },
      Some((ItemKind::Instruction, id)) => {
        let file = env.instruction_file(id);
        if let Some(record) = instruction_lock.instructions.get(id) {
          links.extend(symlink_installs(
            &record.installs,
            &file,
            ItemKind::Instruction,
          ));
        }
        paths.push(file);
      },
      _ => {},
    }
  }
  paths.sort();
  paths.dedup();
  Ok((paths, links))
}

fn symlink_installs(installs: &[InstallRecord], source: &Path, kind: ItemKind) -> Vec<Link> {
  installs
    .iter()
    .filter(|install| {
      let target = Path::new(&install.path);
      is_symlink(target) && symlink_points_to(target, source)
    })
    .map(|install| Link {
      target: install.path.clone(),
      source: source.to_string_lossy().to_string(),
      kind,
    })
    .collect()
}

/// Move a local item to the repository's id: the hub directory or template file, the lock
/// record (name, source, installs) and the symlink installs that point into it.
fn rekey(env: &Env, local_key: &str, remote_key: &str) -> Result<(), String> {
  let (Some((kind, from)), Some((_, to))) = (split_key(local_key), split_key(remote_key)) else {
    return Ok(());
  };
  if from == to {
    return Ok(());
  }
  match kind {
    ItemKind::Skill => {
      let Some(record) = store(env).get(from)? else {
        return Ok(());
      };
      let old_dir = env.hub_dir(from, &record.name);
      let new_dir = env.hub_dir(to, &record.name);
      if env.hub_root.join(to).exists() {
        return Err(format!("{} is already in the hub", to));
      }
      move_dir(&old_dir, &new_dir)?;
      let _ = fs::remove_dir(env.hub_root.join(from));
      invalidate_hash_cache(&old_dir);
      invalidate_hash_cache(&new_dir);
      for install in &record.installs {
        let target = Path::new(&install.path);
        if is_symlink(target) && symlink_points_to(target, &old_dir) {
          write_target(&new_dir, target, InstallMode::Symlink)?;
        }
      }
      store(env).update(|lock| {
        if let Some(record) = lock.skills.remove(from) {
          lock.skills.insert(to.to_string(), record);
        }
        Ok(())
      })
    },
    ItemKind::Instruction => {
      let lock = instruction_service::read_lock(env)?;
      let Some(record) = lock.instructions.get(from) else {
        return Ok(());
      };
      let old_file = env.instruction_file(from);
      let new_file = env.instruction_file(to);
      if new_file.exists() {
        return Err(format!("{} is already in the instruction library", to));
      }
      fs::rename(&old_file, &new_file).map_err(|e| e.to_string())?;
      for install in &record.installs {
        let target = Path::new(&install.path);
        if is_symlink(target) && symlink_points_to(target, &old_file) {
          instruction_service::write_target(&new_file, target, InstallMode::Symlink)?;
        }
      }
      instruction_service::update_lock(env, |lock| {
        if let Some(record) = lock.instructions.remove(from) {
          lock.instructions.insert(to.to_string(), record);
        }
        Ok(())
      })
    },
    ItemKind::Agent | ItemKind::Device => Ok(()),
  }
}

/// A name that no hub skill (or template) has yet: `<name><suffix>`, then numbered.
fn unused_name(name: &str, suffix: &str, taken: &BTreeSet<String>) -> String {
  let mut candidate = format!("{}{}", name, suffix);
  let mut counter = 2;
  while taken.contains(&candidate.to_lowercase()) {
    candidate = format!("{}{}-{}", name, suffix, counter);
    counter += 1;
  }
  candidate
}

/// Set the `name` field of a `SKILL.md` frontmatter, when it has one.
fn rename_frontmatter(skill_md: &Path, name: &str) -> Result<(), String> {
  let Ok(text) = fs::read_to_string(skill_md) else {
    return Ok(());
  };
  let mut lines: Vec<String> = text.split('\n').map(String::from).collect();
  if lines.first().map(|line| line.trim_end()) != Some("---") {
    return Ok(());
  }
  for line in lines.iter_mut().skip(1) {
    if line.trim_end() == "---" {
      break;
    }
    if line.starts_with("name:") {
      *line = format!("name: {}", name);
      return fs::write(skill_md, lines.join("\n")).map_err(|e| e.to_string());
    }
  }
  Ok(())
}

/// Keep the local version of `key` as a new item with `new_id`, a suffixed name and no
/// installs. The installs stay with the id, which gets the repository version.
fn copy_local(env: &Env, key: &str, new_id: &str, detach: bool) -> Result<(), String> {
  match split_key(key) {
    Some((ItemKind::Skill, id)) => {
      let lock = store(env).read()?;
      let Some(record) = lock.skills.get(id) else {
        return Ok(());
      };
      let taken: BTreeSet<String> = lock
        .skills
        .values()
        .map(|record| record.name.to_lowercase())
        .collect();
      let name = unused_name(&record.name, SKILL_COPY_SUFFIX, &taken);
      let dir = env.hub_dir(new_id, &name);
      copy_dir(&env.hub_dir(id, &record.name), &dir, &CopyOpts::all())?;
      rename_frontmatter(&dir.join(SKILL_MD), &name)?;
      invalidate_hash_cache(&dir);
      let hash = hash_dir(&dir)?;
      let copy = SkillRecord {
        name,
        source: if detach {
          SkillSource::None
        } else {
          record.source.clone()
        },
        hash,
        imported_at: record.imported_at.clone(),
        updated_at: now_rfc3339(),
        installs: Vec::new(),
      };
      store(env).update(|lock| {
        lock.skills.insert(new_id.to_string(), copy);
        Ok(())
      })
    },
    Some((ItemKind::Instruction, id)) => {
      let lock = instruction_service::read_lock(env)?;
      let Some(record) = lock.instructions.get(id) else {
        return Ok(());
      };
      let taken: BTreeSet<String> = lock
        .instructions
        .values()
        .map(|record| record.name.to_lowercase())
        .collect();
      let copy = InstructionRecord {
        name: unused_name(&record.name, INSTRUCTION_COPY_SUFFIX, &taken),
        source: if detach {
          InstructionSource::None
        } else {
          record.source.clone()
        },
        hash: record.hash.clone(),
        imported_at: record.imported_at.clone(),
        updated_at: now_rfc3339(),
        installs: Vec::new(),
      };
      fs::copy(env.instruction_file(id), env.instruction_file(new_id))
        .map_err(|e| e.to_string())?;
      instruction_service::update_lock(env, |lock| {
        lock.instructions.insert(new_id.to_string(), copy);
        Ok(())
      })
    },
    _ => Ok(()),
  }
}

pub(super) fn parse_library(content: &[u8]) -> Result<LibraryFile, String> {
  let library: LibraryFile = serde_json::from_slice(content)
    .map_err(|e| format!("The repository's library.json is not valid: {}", e))?;
  if library.format != LIBRARY_FORMAT {
    return Err("The repository is not a YouSkill library".to_string());
  }
  if library.version > LIBRARY_VERSION {
    return Err("The library was written by a newer YouSkill. Update the app first".to_string());
  }
  Ok(library)
}

/// The repository items a restore writes, checked and assembled before anything local
/// changes.
struct Prepared {
  /// Skill ids, each assembled in the staging directory.
  skills: Vec<String>,
  /// Instruction id -> content.
  instructions: BTreeMap<String, Vec<u8>>,
}

/// A repository path split into safe components, or `None` for anything that could leave
/// the directory it is written into.
pub(super) fn safe_components(path: &str) -> Option<Vec<&str>> {
  let parts: Vec<&str> = path.split('/').collect();
  let safe = parts.iter().all(|part| {
    !part.is_empty() && *part != "." && *part != ".." && !part.contains('\\') && !part.contains(':')
  });
  safe.then_some(parts)
}

pub(super) fn write_entry(path: &Path, mode: &str, content: &[u8]) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  if mode == MODE_SYMLINK {
    let target = String::from_utf8_lossy(content).to_string();
    if create_symlink(&target, path).is_ok() {
      return Ok(());
    }
  }
  fs::write(path, content).map_err(|e| e.to_string())?;
  #[cfg(unix)]
  if mode == MODE_EXEC {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
  }
  Ok(())
}

#[cfg(unix)]
fn create_symlink(target: &str, link: &Path) -> std::io::Result<()> {
  std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &str, link: &Path) -> std::io::Result<()> {
  std::os::windows::fs::symlink_file(target.replace('/', "\\"), link)
}

/// Download the repository version of the `keys` items and assemble the skills in staging.
/// Nothing in the hub or the lock files changes yet; any problem with the repository
/// content stops the restore here.
fn prepare_remote_items(
  env: &Env,
  remote: &dyn RemoteRepo,
  planned: &Planned,
  commit: &str,
  keys: &BTreeSet<String>,
) -> Result<Prepared, String> {
  let library = &planned.target_state.library;
  let mut wanted: BTreeMap<String, TreeFile> = BTreeMap::new();
  for key in keys {
    if let (Some(version), Some((_, id))) = (planned.remote.get(key), split_key(key)) {
      wanted.extend(
        version
          .paths(id)
          .into_iter()
          .filter(|(path, _)| planned.target_state.files.contains_key(path)),
      );
    }
  }
  let contents = if wanted.is_empty() {
    BTreeMap::new()
  } else {
    remote.download(commit, &wanted)?
  };
  let mut errors = Vec::new();
  let mut prepared = Prepared {
    skills: Vec::new(),
    instructions: BTreeMap::new(),
  };

  let staging = env.youskill_root.join(STAGING_DIR);
  if staging.exists() {
    remove_path_any(&staging)?;
  }
  fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
  for (path, content) in &contents {
    let Some(parts) = safe_components(path) else {
      errors.push(format!("The repository has an unsafe path: {}", path));
      continue;
    };
    if parts.len() < 4 || parts[0] != SKILLS_DIR {
      continue;
    }
    let (id, name) = (parts[1], parts[2]);
    if !keys.contains(&skill_key(id))
      || library.skills.get(id).map(|skill| skill.name.as_str()) != Some(name)
    {
      continue;
    }
    let mut target = staging.join(id).join(name);
    for part in &parts[3..] {
      target = target.join(part);
    }
    write_entry(&target, &wanted[path].mode, content)?;
  }

  for key in keys {
    match split_key(key) {
      Some((ItemKind::Skill, id)) => {
        let Some(skill) = library.skills.get(id) else {
          continue;
        };
        if validate_skill_name(&skill.name).is_err() {
          errors.push(format!(
            "The repository has a skill with an invalid name: {}",
            skill.name
          ));
        } else if !staging.join(id).join(&skill.name).is_dir() {
          errors.push(format!("The repository has no files for {}", skill.name));
        } else {
          prepared.skills.push(id.to_string());
        }
      },
      Some((ItemKind::Instruction, id)) => {
        let Some(instruction) = library.instructions.get(id) else {
          continue;
        };
        let path = format!("{}/{}.md", INSTRUCTIONS_DIR, id);
        match contents.get(&path) {
          Some(content) if safe_components(&path).is_some() => {
            prepared
              .instructions
              .insert(id.to_string(), content.clone());
          },
          _ => errors.push(format!(
            "The repository has no file for {}",
            instruction.name
          )),
        }
      },
      // A custom agent has no files: its entry is its content.
      Some((ItemKind::Agent | ItemKind::Device, _)) => {},
      None => {},
    }
  }
  if !errors.is_empty() {
    let _ = remove_path_any(&staging);
    return Err(errors.join("\n"));
  }
  Ok(prepared)
}

/// Move the prepared items into the hub and the lock files. Called under the operation
/// lock. Returns whether the custom agent definitions changed.
fn write_prepared(env: &Env, planned: &Planned, prepared: &Prepared) -> Result<(), String> {
  let library = &planned.target_state.library;
  let staging = env.youskill_root.join(STAGING_DIR);
  let lock = store(env).read()?;
  for id in &prepared.skills {
    let skill = &library.skills[id];
    let staged = staging.join(id).join(&skill.name);
    let target = env.hub_dir(id, &skill.name);
    let unchanged = target.is_dir()
      && hash_dir(&target).ok().is_some()
      && hash_dir(&target).ok() == hash_dir(&staged).ok();
    if unchanged {
      continue;
    }
    // A rename moves the hub directory: symlink installs into the old one follow it.
    let mut links = Vec::new();
    if let Some(record) = lock.skills.get(id) {
      let old = env.hub_dir(id, &record.name);
      if record.name != skill.name && old.exists() {
        links = record
          .installs
          .iter()
          .map(|install| PathBuf::from(&install.path))
          .filter(|link| is_symlink(link) && symlink_points_to(link, &old))
          .collect();
        trash_dir(env, &old)?;
      }
    }
    if target.exists() {
      trash_dir(env, &target)?;
    }
    move_dir(&staged, &target)?;
    invalidate_hash_cache(&target);
    for link in &links {
      write_target(&target, link, InstallMode::Symlink)?;
    }
  }

  store(env).update(|lock| {
    for id in &prepared.skills {
      let skill = &library.skills[id];
      match lock.skills.get_mut(id) {
        Some(record) => {
          record.name = skill.name.clone();
          record.source = merged_source(&record.source, &skill.source);
          record.hash = skill.hash.clone();
          record.imported_at = skill.imported_at.clone();
          record.updated_at = skill.updated_at.clone();
        },
        None => {
          lock.skills.insert(
            id.to_string(),
            SkillRecord {
              name: skill.name.clone(),
              source: skill.source.clone(),
              hash: skill.hash.clone(),
              imported_at: skill.imported_at.clone(),
              updated_at: skill.updated_at.clone(),
              installs: Vec::new(),
            },
          );
        },
      }
    }
    Ok(())
  })?;

  for (id, content) in &prepared.instructions {
    let file = env.instruction_file(id);
    if fs::read(&file).ok().as_deref() != Some(content.as_slice()) {
      write_atomic(&file, content)?;
    }
  }
  instruction_service::update_lock(env, |lock| {
    for id in prepared.instructions.keys() {
      let instruction = &library.instructions[id];
      match lock.instructions.get_mut(id) {
        Some(record) => {
          record.name = instruction.name.clone();
          record.source = instruction.source.clone();
          record.hash = instruction.hash.clone();
          record.imported_at = instruction.imported_at.clone();
          record.updated_at = instruction.updated_at.clone();
        },
        None => {
          lock.instructions.insert(
            id.to_string(),
            InstructionRecord {
              name: instruction.name.clone(),
              source: instruction.source.clone(),
              hash: instruction.hash.clone(),
              imported_at: instruction.imported_at.clone(),
              updated_at: instruction.updated_at.clone(),
              installs: Vec::new(),
            },
          );
        },
      }
    }
    Ok(())
  })?;

  let _ = remove_path_any(&staging);
  Ok(())
}

/// Write the custom agents the restore takes or removes into `user_agent_apps.json`, one by
/// one. Returns whether the definitions changed.
fn write_agents(env: &Env, planned: &Planned, actions: &Actions) -> Result<bool, String> {
  let before = read_user_agent_apps(env)?;
  let mut apps = before.clone();
  for key in &actions.writes {
    if let Some(Entry::Agent(agent)) = planned.remote.get(key).map(|version| &version.entry) {
      apps.retain(|app| app.id != agent.app.id);
      apps.push(agent.app.clone());
    }
  }
  for key in &actions.removals {
    if let Some((ItemKind::Agent, id)) = split_key(key) {
      apps.retain(|app| app.id != id);
    }
  }
  apps.sort_by(|a, b| a.id.cmp(&b.id));
  if apps == before {
    return Ok(false);
  }
  write_atomic(&user_agent_apps_path(env), &pretty_json(&apps)?)?;
  Ok(true)
}
