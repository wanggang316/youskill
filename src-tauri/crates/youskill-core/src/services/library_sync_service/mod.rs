//! Back up the library to a private GitHub repository and bring it to other computers.
//!
//! The repository holds the shared library and one device file per computer: skill and
//! instruction files, their metadata with an item scope (`library.json`), the custom agent
//! definitions with their scope (`agents.json`) and `devices/<device-id>.json`, the
//! user-level installs of one device (by agent id and item id, never by path). Install
//! paths, project installs, settings and secrets stay on each computer.
//!
//! A device gets the shared items and its own device-only items (`device_gets`). It writes
//! only those and its own device file; a backup copies the items and device files of other
//! devices from the head unchanged.
//!
//! The local lock files stay the source of truth. A backup exports a snapshot of them and
//! commits it on top of the branch. When another device moved the branch since the last
//! sync, the backup compares each item again against the head and commits on top of it
//! unless an item changed on both sides. A restore (`restore`) merges the branch, or an
//! earlier commit of it, into the hub item by item and commits the merged result on top of
//! the branch. Repository history is never rewritten.

use crate::models::{
  AgentApp, InstallMode, InstallRecord, InstallRequest, InstallScope, InstallTargetSpec,
  InstructionSource, SkillRecord, SkillSource, SyncAction, TargetState, UninstallRequest,
};
use crate::services::drift_service::build_view;
use crate::services::env::Env;
use crate::services::hub_service::{self, trash_dir};
use crate::services::install_service;
use crate::services::instruction_service;
use crate::services::lock_service::{ops_guard, store};
use crate::utils::folder::move_dir;
use crate::utils::git_remote::{
  self, blob_sha, GithubRepo, RemoteRepo, TreeFile, UpdateError, MODE_EXEC, MODE_FILE, MODE_SYMLINK,
};
use crate::utils::hash::{hash_dir, invalidate_hash_cache, is_excluded_component};
use crate::utils::path::{remove_path_any, validate_skill_name};
use crate::utils::time::now_rfc3339;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

mod checkpoint;
mod migrate;
mod restore;

pub use crate::utils::git_remote::RemoteCommit;
pub use checkpoint::{undo_restore, CheckpointInfo};
pub use migrate::{migrate, migrate_with};
pub use restore::{
  apply_restore, apply_restore_with, history, history_with, plan_restore, plan_restore_with,
  ConflictChoice, ItemChange, MatchReason, PlanState, RemovalChoice, Resolution, RestoreDecisions,
  RestoreItem, RestoreMode, RestoreOptions, RestoreOutcome, RestorePlan, RestoreResult,
};

/// Local sync state, next to the lock files. Never synced.
pub const SYNC_STATE_FILE: &str = ".sync-state.json";
/// Where pulled skills are assembled before they move into the hub (same volume).
const STAGING_DIR: &str = ".sync-staging";
pub const DEFAULT_REPO_NAME: &str = "youskill-library";

const LIBRARY_FILE: &str = "library.json";
const AGENTS_FILE: &str = "agents.json";
const README_FILE: &str = "README.md";
const GITATTRIBUTES_FILE: &str = ".gitattributes";
const SKILLS_DIR: &str = "skills";
const INSTRUCTIONS_DIR: &str = "instructions";
const DEVICES_DIR: &str = "devices";
const USER_AGENT_APPS_FILE: &str = "user_agent_apps.json";
const LIBRARY_FORMAT: &str = "youskill-library";
/// Version 2: item scopes and device files. Releases up to 0.10.0 refuse a library above
/// version 1, so they never write to a migrated repository.
const LIBRARY_VERSION: u32 = 2;
/// GitHub refuses files over 100 MB; a skill with a file over this stays on this computer.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Shared metadata (`library.json`, `agents.json`) and device files
// ---------------------------------------------------------------------------

/// Which devices get an item: every device, or one device by id.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
  #[default]
  Shared,
  Device(String),
}

/// Whether `device` gets an item of `scope`: shared items and its own device-only items.
/// Every selection of the items of a device (snapshot, plan, restore, pending) goes through
/// this function, so a device never takes or changes the device-only items of another one.
pub(crate) fn device_gets(scope: &Scope, device: &str) -> bool {
  match scope {
    Scope::Shared => true,
    Scope::Device(owner) => owner == device,
  }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFile {
  pub format: String,
  pub version: u32,
  #[serde(default)]
  pub skills: BTreeMap<String, LibrarySkill>,
  #[serde(default)]
  pub instructions: BTreeMap<String, LibraryInstruction>,
}

impl Default for LibraryFile {
  fn default() -> Self {
    Self {
      format: LIBRARY_FORMAT.to_string(),
      version: LIBRARY_VERSION,
      skills: BTreeMap::new(),
      instructions: BTreeMap::new(),
    }
  }
}

impl LibraryFile {
  /// Remove the paths of local sources. Every library read from the repository or the sync
  /// state goes through this, so a path that an earlier version wrote is never carried,
  /// compared or written again.
  fn without_local_paths(mut self) -> Self {
    for skill in self.skills.values_mut() {
      skill.source = shared_source(&skill.source);
    }
    for instruction in self.instructions.values_mut() {
      instruction.source = shared_instruction_source(&instruction.source);
    }
    self
  }
}

/// A skill record. A version 1 record also has `agents`, which is ignored: read as
/// version 2, every version 1 item is shared and has no installs.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySkill {
  pub name: String,
  #[serde(default)]
  pub source: SkillSource,
  pub hash: String,
  pub imported_at: String,
  pub updated_at: String,
  #[serde(default)]
  pub scope: Scope,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInstruction {
  pub name: String,
  #[serde(default)]
  pub source: InstructionSource,
  pub hash: String,
  pub imported_at: String,
  pub updated_at: String,
  #[serde(default)]
  pub scope: Scope,
}

/// A custom agent definition in `agents.json`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SyncedAgent {
  #[serde(flatten)]
  pub app: AgentApp,
  #[serde(default)]
  pub scope: Scope,
}

/// One user-level install in a device file.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInstall {
  pub kind: ItemKind,
  pub id: String,
}

/// `devices/<device-id>.json`: the user-level installs of one device. Only that device
/// writes it.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFile {
  /// Display name, from the computer name. Not an identity.
  pub name: String,
  /// Installed items by agent id.
  #[serde(default)]
  pub installs: BTreeMap<String, Vec<DeviceInstall>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_backup_at: Option<String>,
}

impl DeviceFile {
  /// Agents by item key.
  fn by_item(&self) -> BTreeMap<String, Vec<String>> {
    let mut items: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (agent, installs) in &self.installs {
      for install in installs {
        let key = match install.kind {
          ItemKind::Skill => skill_key(&install.id),
          ItemKind::Instruction => instruction_key(&install.id),
          _ => continue,
        };
        items.entry(key).or_default().push(agent.clone());
      }
    }
    for agents in items.values_mut() {
      agents.sort();
      agents.dedup();
    }
    items
  }

  /// Installs by agent from agents by item key.
  fn installs_from(items: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<DeviceInstall>> {
    let mut installs: BTreeMap<String, Vec<DeviceInstall>> = BTreeMap::new();
    for (key, agents) in items {
      let Some((kind, id)) = split_key(key) else {
        continue;
      };
      for agent in agents {
        installs
          .entry(agent.clone())
          .or_default()
          .push(DeviceInstall {
            kind,
            id: id.to_string(),
          });
      }
    }
    for list in installs.values_mut() {
      list.sort();
      list.dedup();
    }
    installs
  }

  /// Equal except the backup time.
  fn same_installs(&self, other: &DeviceFile) -> bool {
    self.name == other.name && self.installs == other.installs
  }
}

fn device_path(device: &str) -> String {
  format!("{}/{}.json", DEVICES_DIR, device)
}

// ---------------------------------------------------------------------------
// Local state
// ---------------------------------------------------------------------------

/// A head-only change that a backup on a moved head kept: the item is not written here,
/// later backups carry `remote` forward, and the next restore offers it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IncomingItem {
  /// The item at the base before the head changed it.
  #[serde(default)]
  base: Option<ItemVersion>,
  /// The repository version; `None` when the repository removed the item.
  #[serde(default)]
  remote: Option<ItemVersion>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
  /// `owner/name`; `None` when not connected.
  #[serde(default)]
  pub repo: Option<String>,
  #[serde(default)]
  pub branch: Option<String>,
  #[serde(default)]
  pub device_name: Option<String>,
  /// The random id of this device; its device file is `devices/<id>.json`.
  #[serde(default)]
  pub device_id: Option<String>,
  /// The commit this computer last backed up or pulled: the base of the next sync.
  #[serde(default)]
  pub last_synced_commit: Option<String>,
  #[serde(default)]
  pub last_backup_at: Option<String>,
  #[serde(default)]
  pub last_pull_at: Option<String>,
  /// Files of the base commit.
  #[serde(default)]
  pub base_files: BTreeMap<String, TreeFile>,
  #[serde(default)]
  pub base_library: Option<LibraryFile>,
  /// Custom agents of the base commit; `None` for a state written before version 2, read
  /// from the repository on the next sync.
  #[serde(default)]
  pub base_agents: Option<Vec<SyncedAgent>>,
  /// This device's file in the base commit.
  #[serde(default)]
  pub base_device: Option<DeviceFile>,
  /// User-level agents installed on this computer at the base, by `skill:<id>` /
  /// `instruction:<id>`. A snapshot applies the install changes since then to the device
  /// file of the base.
  #[serde(default)]
  pub base_local_agents: BTreeMap<String, Vec<String>>,
  /// Remote items the user left out of a restore, by item key, with the version they had.
  /// A backup carries this version forward instead of the local one.
  #[serde(default)]
  pub deferred: BTreeMap<String, ItemVersion>,
  /// Head-only changes kept by a backup on a moved head, by item key. Carried like a
  /// deferred item, but offered selected by the next restore.
  #[serde(default)]
  pub incoming: BTreeMap<String, IncomingItem>,
}

impl SyncState {
  fn connected(&self) -> Result<(String, String), String> {
    match (&self.repo, &self.branch) {
      (Some(repo), Some(branch)) => Ok((repo.clone(), branch.clone())),
      _ => Err("The library is not connected to a GitHub repository".to_string()),
    }
  }

  fn reset_base(&mut self) {
    self.last_synced_commit = None;
    self.last_backup_at = None;
    self.last_pull_at = None;
    self.base_files.clear();
    self.base_library = None;
    self.base_agents = None;
    self.base_device = None;
    self.base_local_agents.clear();
    self.deferred.clear();
    self.incoming.clear();
  }

  /// The device id, or an empty id (which owns no item) before the first sync.
  fn device(&self) -> String {
    self.device_id.clone().unwrap_or_default()
  }

  fn device_label(&self) -> String {
    self
      .device_name
      .clone()
      .filter(|name| !name.trim().is_empty())
      .unwrap_or_else(default_device_name)
  }

  /// The base commit as a repository state.
  fn base(&self) -> RepoState {
    RepoState {
      library: self.base_library.clone().unwrap_or_default(),
      agents: self.base_agents.clone().unwrap_or_default(),
      files: self.base_files.clone(),
      device: self.base_device.clone(),
    }
  }
}

fn state_path(env: &Env) -> PathBuf {
  env.youskill_root.join(SYNC_STATE_FILE)
}

pub fn read_state(env: &Env) -> Result<SyncState, String> {
  let path = state_path(env);
  if !path.is_file() {
    return Ok(SyncState::default());
  }
  let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
  if content.trim().is_empty() {
    return Ok(SyncState::default());
  }
  let mut state: SyncState = serde_json::from_str(&content)
    .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
  // A deferred `agents.json` of a version 1 state; custom agents are items one by one now.
  state
    .deferred
    .retain(|key, version| split_key(key).is_some() && version.entry != Entry::Legacy);
  state.base_library = state.base_library.map(LibraryFile::without_local_paths);
  for version in state.deferred.values_mut() {
    version.strip_local_paths();
  }
  for incoming in state.incoming.values_mut() {
    for version in [&mut incoming.base, &mut incoming.remote]
      .into_iter()
      .flatten()
    {
      version.strip_local_paths();
    }
  }
  Ok(state)
}

fn write_state(env: &Env, state: &SyncState) -> Result<(), String> {
  let path = state_path(env);
  write_atomic(
    &path,
    serde_json::to_string_pretty(state)
      .map_err(|e| e.to_string())?
      .as_bytes(),
  )
}

/// Give this computer a device id on its first connect or first sync after the update.
fn ensure_device(env: &Env, state: &mut SyncState) -> Result<String, String> {
  if let Some(id) = &state.device_id {
    return Ok(id.clone());
  }
  let id = uuid::Uuid::new_v4().to_string();
  state.device_id = Some(id.clone());
  write_state(env, state)?;
  Ok(id)
}

/// Read the custom agents of a base written before version 2 from the repository.
fn fill_base_agents(
  env: &Env,
  remote: &dyn RemoteRepo,
  state: &mut SyncState,
) -> Result<(), String> {
  if state.base_agents.is_some() {
    return Ok(());
  }
  let Some(commit) = state.last_synced_commit.clone() else {
    return Ok(());
  };
  let agents = match state.base_files.get(AGENTS_FILE) {
    Some(file) => {
      let wanted = BTreeMap::from([(AGENTS_FILE.to_string(), file.clone())]);
      parse_agents(&remote.download(&commit, &wanted)?[AGENTS_FILE])?
    },
    None => Vec::new(),
  };
  state.base_agents = Some(agents);
  write_state(env, state)
}

fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  let tmp = path.with_extension("sync-tmp");
  fs::write(&tmp, content).map_err(|e| e.to_string())?;
  fs::rename(&tmp, path).map_err(|e| {
    let _ = fs::remove_file(&tmp);
    e.to_string()
  })
}

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
  Skill,
  Instruction,
  /// A custom agent definition.
  Agent,
  /// This device's installs.
  Device,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
  Added,
  Modified,
  Removed,
}

/// One library item that differs between two states of the library.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryChange {
  pub kind: ItemKind,
  pub id: String,
  pub name: String,
  pub change: ChangeKind,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
  pub connected: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub repo: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub branch: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub device_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_backup_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_pull_at: Option<String>,
  /// Whether this computer has backed up or pulled at least once.
  pub has_synced: bool,
  /// The last synced commit holds a version 1 library: the next backup or restore asks to
  /// migrate the repository, unless another device migrated it already.
  #[serde(default)]
  pub legacy_base: bool,
  /// Changes on this computer since the last sync.
  pub changes: Vec<LibraryChange>,
  /// Skills left out of the backup (a file over 50 MB, a missing hub copy).
  pub skipped: Vec<String>,
  pub library_empty: bool,
  /// The checkpoint of the last restore, which `undo_restore` puts back.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub checkpoint: Option<CheckpointInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupResult {
  Pushed,
  UpToDate,
  /// The repository has commits this computer has not pulled; nothing changed here.
  RemoteAhead,
  /// An item changed here and in the repository since the last sync.
  Diverged,
  /// The repository already holds a library this computer never synced with.
  RemoteNotEmpty,
  /// The repository holds a version 1 library; migrate it first (`migrate`).
  MigrationRequired,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupOutcome {
  pub result: BackupResult,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub commit: Option<String>,
  pub changes: Vec<LibraryChange>,
  pub skipped: Vec<String>,
  /// Items another device changed that the backup kept for the next restore.
  #[serde(default)]
  pub incoming: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PendingAction {
  /// In this device file, not installed for this agent here.
  Install,
  /// Installed here, not in this device file.
  Uninstall,
  /// A copy install behind the hub.
  Update,
}

/// An install change that would bring this computer in line with its device file.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingItem {
  pub kind: ItemKind,
  pub id: String,
  pub name: String,
  pub action: PendingAction,
  pub agent_id: String,
  /// Install path, for `update`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPendingOutcome {
  pub applied: usize,
  pub failures: Vec<PendingFailure>,
}

/// A pending item that was not applied, and why.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingFailure {
  pub kind: ItemKind,
  pub name: String,
  pub reason: String,
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

/// The library entry of an item.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
  Skill(LibrarySkill),
  Instruction(LibraryInstruction),
  Agent(SyncedAgent),
  /// The whole `agents.json` of a version 1 sync state.
  #[serde(other)]
  Legacy,
}

/// One item in one state of the library: its entry and its files, by path below the item
/// (inside `skills/<id>/<name>/`; empty for a one-file item). A custom agent has no files.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ItemVersion {
  entry: Entry,
  files: BTreeMap<String, TreeFile>,
}

impl ItemVersion {
  fn kind(&self) -> ItemKind {
    match self.entry {
      Entry::Skill(_) => ItemKind::Skill,
      Entry::Instruction(_) => ItemKind::Instruction,
      Entry::Agent(_) | Entry::Legacy => ItemKind::Agent,
    }
  }

  fn name(&self) -> String {
    match &self.entry {
      Entry::Skill(skill) => skill.name.clone(),
      Entry::Instruction(instruction) => instruction.name.clone(),
      Entry::Agent(agent) => agent.app.display_name.clone(),
      Entry::Legacy => String::new(),
    }
  }

  fn same_content(&self, other: &ItemVersion) -> bool {
    self == other
  }

  /// Equal files, name and source: two versions that only differ in hash and timestamps,
  /// as two computers that made the same edit produce.
  fn same_except_times(&self, other: &ItemVersion) -> bool {
    let strip = |entry: &Entry| match entry.clone() {
      Entry::Skill(skill) => Entry::Skill(LibrarySkill {
        hash: String::new(),
        imported_at: String::new(),
        updated_at: String::new(),
        ..skill
      }),
      Entry::Instruction(instruction) => Entry::Instruction(LibraryInstruction {
        hash: String::new(),
        imported_at: String::new(),
        updated_at: String::new(),
        ..instruction
      }),
      other => other,
    };
    self.files == other.files && strip(&self.entry) == strip(&other.entry)
  }

  /// The source key that pairs items on a first merge: a GitHub source. A folder path is
  /// local to one computer, so equal paths on two computers are not the same source.
  fn source_key(&self) -> Option<String> {
    match &self.entry {
      Entry::Skill(skill) => match &skill.source {
        SkillSource::Github { .. } => Some(skill.source.key()),
        _ => None,
      },
      Entry::Instruction(instruction) => match &instruction.source {
        InstructionSource::Github {
          repo,
          file_path,
          branch,
        } => Some(format!(
          "github:{}@{}:{}",
          repo.trim().to_ascii_lowercase(),
          branch.as_deref().unwrap_or_default(),
          file_path.trim_matches('/')
        )),
        _ => None,
      },
      Entry::Agent(_) | Entry::Legacy => None,
    }
  }

  /// Remove the path of a local source (see `LibraryFile::without_local_paths`).
  fn strip_local_paths(&mut self) {
    match &mut self.entry {
      Entry::Skill(skill) => skill.source = shared_source(&skill.source),
      Entry::Instruction(instruction) => {
        instruction.source = shared_instruction_source(&instruction.source)
      },
      Entry::Agent(_) | Entry::Legacy => {},
    }
  }

  /// Repository paths of the item's files.
  fn paths(&self, id: &str) -> BTreeMap<String, TreeFile> {
    self
      .files
      .iter()
      .filter_map(|(relative, file)| {
        let path = match &self.entry {
          Entry::Skill(skill) => format!("{}/{}/{}/{}", SKILLS_DIR, id, skill.name, relative),
          Entry::Instruction(_) => format!("{}/{}.md", INSTRUCTIONS_DIR, id),
          Entry::Agent(_) | Entry::Legacy => return None,
        };
        Some((path, file.clone()))
      })
      .collect()
  }
}

fn split_key(key: &str) -> Option<(ItemKind, &str)> {
  if let Some(id) = key.strip_prefix("skill:") {
    return Some((ItemKind::Skill, id));
  }
  if let Some(id) = key.strip_prefix("instruction:") {
    return Some((ItemKind::Instruction, id));
  }
  key.strip_prefix("agent:").map(|id| (ItemKind::Agent, id))
}

fn skill_key(id: &str) -> String {
  format!("skill:{}", id)
}

fn instruction_key(id: &str) -> String {
  format!("instruction:{}", id)
}

fn agent_key(id: &str) -> String {
  format!("agent:{}", id)
}

/// The items `device` gets from one state of the library, by item key.
fn item_versions(
  library: &LibraryFile,
  agents: &[SyncedAgent],
  files: &BTreeMap<String, TreeFile>,
  device: &str,
) -> BTreeMap<String, ItemVersion> {
  let mut items = BTreeMap::new();
  for (id, skill) in &library.skills {
    if !device_gets(&skill.scope, device) {
      continue;
    }
    let prefix = format!("{}/{}/{}/", SKILLS_DIR, id, skill.name);
    let item_files = files
      .range(prefix.clone()..)
      .take_while(|(path, _)| path.starts_with(&prefix))
      .map(|(path, file)| (path[prefix.len()..].to_string(), file.clone()))
      .collect();
    items.insert(
      skill_key(id),
      ItemVersion {
        entry: Entry::Skill(skill.clone()),
        files: item_files,
      },
    );
  }
  for (id, instruction) in &library.instructions {
    if !device_gets(&instruction.scope, device) {
      continue;
    }
    let path = format!("{}/{}.md", INSTRUCTIONS_DIR, id);
    let item_files = files
      .get(&path)
      .map(|file| (String::new(), file.clone()))
      .into_iter()
      .collect();
    items.insert(
      instruction_key(id),
      ItemVersion {
        entry: Entry::Instruction(instruction.clone()),
        files: item_files,
      },
    );
  }
  for agent in agents {
    if !device_gets(&agent.scope, device) {
      continue;
    }
    items.insert(
      agent_key(&agent.app.id),
      ItemVersion {
        entry: Entry::Agent(agent.clone()),
        files: BTreeMap::new(),
      },
    );
  }
  items
}

/// One state of the repository: a commit, or the base kept in the sync state.
#[derive(Debug, Clone, Default)]
struct RepoState {
  library: LibraryFile,
  agents: Vec<SyncedAgent>,
  files: BTreeMap<String, TreeFile>,
  /// This device's file.
  device: Option<DeviceFile>,
}

/// The parts of a repository state that belong to other devices: their device-only items
/// and device files. A backup copies them unchanged.
#[derive(Debug, Default)]
struct Foreign {
  skills: BTreeMap<String, LibrarySkill>,
  instructions: BTreeMap<String, LibraryInstruction>,
  agents: Vec<SyncedAgent>,
  files: BTreeMap<String, TreeFile>,
}

impl RepoState {
  fn items(&self, device: &str) -> BTreeMap<String, ItemVersion> {
    item_versions(&self.library, &self.agents, &self.files, device)
  }

  fn foreign(&self, device: &str) -> Foreign {
    let mut foreign = Foreign::default();
    let mut prefixes = Vec::new();
    for (id, skill) in &self.library.skills {
      if !device_gets(&skill.scope, device) {
        foreign.skills.insert(id.clone(), skill.clone());
        prefixes.push(format!("{}/{}/", SKILLS_DIR, id));
      }
    }
    for (id, instruction) in &self.library.instructions {
      if !device_gets(&instruction.scope, device) {
        foreign.instructions.insert(id.clone(), instruction.clone());
        prefixes.push(format!("{}/{}.md", INSTRUCTIONS_DIR, id));
      }
    }
    foreign.agents = self
      .agents
      .iter()
      .filter(|agent| !device_gets(&agent.scope, device))
      .cloned()
      .collect();
    let own_device = device_path(device);
    prefixes.push(format!("{}/", DEVICES_DIR));
    for (path, file) in &self.files {
      if path != &own_device && prefixes.iter().any(|prefix| path.starts_with(prefix)) {
        foreign.files.insert(path.clone(), file.clone());
      }
    }
    foreign
  }
}

/// The library, custom agents and this device's file of a commit; `None` when the commit
/// holds no library.
fn load_repo_state(
  remote: &dyn RemoteRepo,
  commit: &str,
  files: &BTreeMap<String, TreeFile>,
  device: &str,
) -> Result<Option<RepoState>, String> {
  if !files.contains_key(LIBRARY_FILE) {
    return Ok(None);
  }
  let own_device = device_path(device);
  let wanted: BTreeMap<String, TreeFile> = files
    .iter()
    .filter(|(path, _)| {
      path.as_str() == LIBRARY_FILE
        || path.as_str() == AGENTS_FILE
        || (!device.is_empty() && **path == own_device)
    })
    .map(|(path, file)| (path.clone(), file.clone()))
    .collect();
  let contents = remote.download(commit, &wanted)?;
  let library = restore::parse_library(&contents[LIBRARY_FILE])?;
  let agents = match contents.get(AGENTS_FILE) {
    Some(content) => parse_agents(content)?,
    None => Vec::new(),
  };
  let device = match contents.get(&own_device) {
    Some(content) => Some(
      serde_json::from_slice(content)
        .map_err(|e| format!("The repository's {} is not valid: {}", own_device, e))?,
    ),
    None => None,
  };
  Ok(Some(RepoState {
    library,
    agents,
    files: files.clone(),
    device,
  }))
}

fn parse_agents(content: &[u8]) -> Result<Vec<SyncedAgent>, String> {
  let mut agents: Vec<SyncedAgent> = serde_json::from_slice(content)
    .map_err(|e| format!("The repository's agents.json is not valid: {}", e))?;
  agents.sort_by(|a, b| a.app.id.cmp(&b.app.id));
  Ok(agents)
}

/// Agents installed at user level on this computer.
fn user_agents(installs: &[InstallRecord]) -> Vec<String> {
  let set: BTreeSet<String> = installs
    .iter()
    .filter(|install| install.scope == InstallScope::User)
    .flat_map(|install| install.agent_ids.iter().cloned())
    .collect();
  set.into_iter().collect()
}

/// The agents of an item in this device file after this computer's own install changes
/// since the base: an agent installed here is added, one uninstalled here is removed, and
/// the others stay as the base device file has them. Without a device file at the base the
/// local installs are the device file.
fn effective_agents(
  base_agents: Option<&Vec<String>>,
  base_local: Option<&Vec<String>>,
  now_local: &[String],
) -> Vec<String> {
  let Some(base_agents) = base_agents else {
    return now_local.to_vec();
  };
  let empty = Vec::new();
  let base_local = base_local.unwrap_or(&empty);
  let mut set: BTreeSet<String> = base_agents.iter().cloned().collect();
  for agent in base_local {
    if !now_local.contains(agent) {
      set.remove(agent);
    }
  }
  for agent in now_local {
    if !base_local.contains(agent) {
      set.insert(agent.clone());
    }
  }
  set.into_iter().collect()
}

/// The source a backup writes: a GitHub source without the update check fields, a local
/// source (`folder`, `zip`) without its path. The path is valid only on the computer that
/// imported the item.
fn shared_source(source: &SkillSource) -> SkillSource {
  match source {
    SkillSource::Folder { .. } => SkillSource::Folder {
      path: String::new(),
    },
    SkillSource::Zip { .. } => SkillSource::Zip {
      path: String::new(),
    },
    other => without_check_fields(other),
  }
}

/// The instruction source a backup writes: a local source (`file`, `agent`) without its
/// path.
fn shared_instruction_source(source: &InstructionSource) -> InstructionSource {
  match source {
    InstructionSource::File { .. } => InstructionSource::File {
      path: String::new(),
    },
    InstructionSource::Agent { .. } => InstructionSource::Agent {
      path: String::new(),
    },
    other => other.clone(),
  }
}

/// A GitHub source without the fields `check_source_updates` keeps refreshing.
fn without_check_fields(source: &SkillSource) -> SkillSource {
  match source {
    SkillSource::Github {
      repo,
      url,
      skill_path,
      branch,
      remote_sha,
      marketplace_id,
      ..
    } => SkillSource::Github {
      repo: repo.clone(),
      url: url.clone(),
      skill_path: skill_path.clone(),
      branch: branch.clone(),
      remote_sha: remote_sha.clone(),
      marketplace_id: marketplace_id.clone(),
      latest_remote_sha: None,
      checked_at: None,
    },
    other => other.clone(),
  }
}

/// A pulled source. A GitHub source keeps this computer's last update check when the
/// location is the same. A local source keeps this computer's path when the kind is the
/// same, else it has no path: a path in the repository (written before paths stayed local)
/// belongs to another computer. A new item has `SkillSource::None` as `local`.
fn merged_source(local: &SkillSource, remote: &SkillSource) -> SkillSource {
  match (local, remote) {
    (SkillSource::Folder { .. }, SkillSource::Folder { .. })
    | (SkillSource::Zip { .. }, SkillSource::Zip { .. }) => local.clone(),
    (_, SkillSource::Folder { .. } | SkillSource::Zip { .. }) => shared_source(remote),
    (
      SkillSource::Github {
        latest_remote_sha,
        checked_at,
        ..
      },
      SkillSource::Github {
        repo,
        url,
        skill_path,
        branch,
        remote_sha,
        marketplace_id,
        ..
      },
    ) if local.same_location(remote) => SkillSource::Github {
      repo: repo.clone(),
      url: url.clone(),
      skill_path: skill_path.clone(),
      branch: branch.clone(),
      remote_sha: remote_sha.clone(),
      marketplace_id: marketplace_id.clone(),
      latest_remote_sha: latest_remote_sha.clone(),
      checked_at: checked_at.clone(),
    },
    _ => remote.clone(),
  }
}

/// A pulled instruction source: a local source keeps this computer's path when the kind is
/// the same, else it has no path (see `merged_source`).
fn merged_instruction_source(
  local: &InstructionSource,
  remote: &InstructionSource,
) -> InstructionSource {
  match (local, remote) {
    (InstructionSource::File { .. }, InstructionSource::File { .. })
    | (InstructionSource::Agent { .. }, InstructionSource::Agent { .. }) => local.clone(),
    _ => shared_instruction_source(remote),
  }
}

/// The source column of the README: the source key without a local path.
fn readme_source(source: &SkillSource) -> String {
  match source {
    SkillSource::Folder { .. } => "folder".to_string(),
    other => other.key(),
  }
}

/// Every file of a skill directory as `(relative path, mode, content)`. `Err` names a file
/// that is too large to back up.
fn skill_files(dir: &Path) -> Result<Vec<(String, String, Vec<u8>)>, String> {
  let mut files = Vec::new();
  let walker = WalkDir::new(dir)
    .follow_links(false)
    .min_depth(1)
    .sort_by_file_name()
    .into_iter()
    .filter_entry(|entry| !is_excluded_component(&entry.file_name().to_string_lossy()));
  for entry in walker {
    let entry = entry.map_err(|e| e.to_string())?;
    let relative = entry
      .path()
      .strip_prefix(dir)
      .map_err(|e| e.to_string())?
      .to_string_lossy()
      .replace('\\', "/");
    let file_type = entry.file_type();
    if file_type.is_symlink() {
      let target = fs::read_link(entry.path()).map_err(|e| e.to_string())?;
      files.push((
        relative,
        MODE_SYMLINK.to_string(),
        target.to_string_lossy().replace('\\', "/").into_bytes(),
      ));
    } else if file_type.is_file() {
      let meta = entry.metadata().map_err(|e| e.to_string())?;
      if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is larger than 50 MB", relative));
      }
      let content = fs::read(entry.path()).map_err(|e| e.to_string())?;
      files.push((relative, file_mode(&meta).to_string(), content));
    }
  }
  Ok(files)
}

#[cfg(unix)]
fn file_mode(meta: &fs::Metadata) -> &'static str {
  use std::os::unix::fs::PermissionsExt;
  if meta.permissions().mode() & 0o111 != 0 {
    MODE_EXEC
  } else {
    MODE_FILE
  }
}

#[cfg(not(unix))]
fn file_mode(_meta: &fs::Metadata) -> &'static str {
  MODE_FILE
}

fn user_agent_apps_path(env: &Env) -> PathBuf {
  env.config_dir.join(USER_AGENT_APPS_FILE)
}

fn read_user_agent_apps(env: &Env) -> Result<Vec<AgentApp>, String> {
  let path = user_agent_apps_path(env);
  if !path.is_file() {
    return Ok(Vec::new());
  }
  let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
  if content.trim().is_empty() {
    return Ok(Vec::new());
  }
  let mut apps: Vec<AgentApp> = serde_json::from_str(&content)
    .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
  apps.sort_by(|a, b| a.id.cmp(&b.id));
  Ok(apps)
}

fn pretty_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
  let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
  text.push('\n');
  Ok(text.into_bytes())
}

fn table_cell(text: &str) -> String {
  text.replace('|', "\\|").replace('\n', " ")
}

fn readme(library: &LibraryFile) -> Vec<u8> {
  let mut text = String::from(
    "# YouSkill library\n\nThis repository is the skill library of \
     [YouSkill](https://github.com/wanggang316/you-skill). YouSkill writes it on every \
     backup; change skills in the app, then back up again.\n",
  );
  let mut skills: Vec<&LibrarySkill> = library.skills.values().collect();
  skills.sort_by_key(|skill| skill.name.to_lowercase());
  if !skills.is_empty() {
    text.push_str("\n## Skills\n\n| Skill | Source |\n| --- | --- |\n");
    for skill in skills {
      text.push_str(&format!(
        "| {} | {} |\n",
        table_cell(&skill.name),
        table_cell(&readme_source(&skill.source))
      ));
    }
  }
  let mut instructions: Vec<&LibraryInstruction> = library.instructions.values().collect();
  instructions.sort_by_key(|instruction| instruction.name.to_lowercase());
  if !instructions.is_empty() {
    text.push_str("\n## Instructions\n\n");
    for instruction in instructions {
      text.push_str(&format!("- {}\n", instruction.name.replace('\n', " ")));
    }
  }
  text.into_bytes()
}

// ---------------------------------------------------------------------------
// Snapshot of this computer's library
// ---------------------------------------------------------------------------

/// This device's part of the library: the items it gets and its device file.
struct Snapshot {
  device_id: String,
  /// Repository path -> (mode, content) of the item files.
  contents: BTreeMap<String, (String, Vec<u8>)>,
  /// Item files carried forward from the repository without local content.
  carried: BTreeMap<String, TreeFile>,
  library: LibraryFile,
  agents: Vec<SyncedAgent>,
  device: DeviceFile,
  local_agents: BTreeMap<String, Vec<String>>,
  skipped: Vec<String>,
}

impl Snapshot {
  fn item_tree(&self) -> BTreeMap<String, TreeFile> {
    let mut tree = self.carried.clone();
    tree.extend(self.contents.iter().map(|(path, (mode, content))| {
      (
        path.clone(),
        TreeFile {
          mode: mode.clone(),
          sha: blob_sha(content),
        },
      )
    }));
    tree
  }

  fn items(&self) -> BTreeMap<String, ItemVersion> {
    item_versions(
      &self.library,
      &self.agents,
      &self.item_tree(),
      &self.device_id,
    )
  }

  /// Replace the local version of an item with a repository version (`None`: without the
  /// item), so it is neither a local change nor reverted by the next backup.
  fn carry(&mut self, key: &str, version: Option<&ItemVersion>) {
    let Some((kind, id)) = split_key(key) else {
      return;
    };
    match kind {
      ItemKind::Skill => {
        self.library.skills.remove(id);
        let prefix = format!("{}/{}/", SKILLS_DIR, id);
        self.contents.retain(|path, _| !path.starts_with(&prefix));
        self.carried.retain(|path, _| !path.starts_with(&prefix));
      },
      ItemKind::Instruction => {
        self.library.instructions.remove(id);
        let path = format!("{}/{}.md", INSTRUCTIONS_DIR, id);
        self.contents.remove(&path);
        self.carried.remove(&path);
      },
      ItemKind::Agent => self.agents.retain(|agent| agent.app.id != id),
      ItemKind::Device => return,
    }
    let Some(version) = version else {
      return;
    };
    match &version.entry {
      Entry::Skill(skill) => {
        self.library.skills.insert(id.to_string(), skill.clone());
      },
      Entry::Instruction(instruction) => {
        self
          .library
          .instructions
          .insert(id.to_string(), instruction.clone());
      },
      Entry::Agent(agent) => {
        self.agents.push(agent.clone());
        self.agents.sort_by(|a, b| a.app.id.cmp(&b.app.id));
      },
      Entry::Legacy => {},
    }
    self.carried.extend(version.paths(id));
  }
}

/// The scope of a local item: the scope the base gives it when this device gets it there,
/// else shared (a new item is shared).
fn local_scope(base: Option<&Scope>, device: &str) -> Scope {
  base
    .filter(|scope| device_gets(scope, device))
    .cloned()
    .unwrap_or_default()
}

fn local_snapshot(env: &Env, state: &SyncState, device: &str) -> Result<Snapshot, String> {
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  let base = state.base_library.as_ref();
  let mut snapshot = Snapshot {
    device_id: device.to_string(),
    contents: BTreeMap::new(),
    carried: BTreeMap::new(),
    library: LibraryFile::default(),
    agents: Vec::new(),
    device: DeviceFile::default(),
    local_agents: BTreeMap::new(),
    skipped: Vec::new(),
  };

  for (id, record) in &lock.skills {
    if record.name.is_empty() || validate_skill_name(&record.name).is_err() {
      snapshot.skipped.push(id.clone());
      continue;
    }
    let dir = env.hub_dir(id, &record.name);
    if !dir.is_dir() {
      snapshot.skipped.push(record.name.clone());
      continue;
    }
    let files = match skill_files(&dir) {
      Ok(files) => files,
      Err(_) => {
        snapshot.skipped.push(record.name.clone());
        continue;
      },
    };
    for (relative, mode, content) in files {
      snapshot.contents.insert(
        format!("{}/{}/{}/{}", SKILLS_DIR, id, record.name, relative),
        (mode, content),
      );
    }
    snapshot.library.skills.insert(
      id.clone(),
      LibrarySkill {
        name: record.name.clone(),
        source: shared_source(&record.source),
        hash: record.hash.clone(),
        imported_at: record.imported_at.clone(),
        updated_at: record.updated_at.clone(),
        scope: local_scope(
          base
            .and_then(|library| library.skills.get(id))
            .map(|skill| &skill.scope),
          device,
        ),
      },
    );
    let local = user_agents(&record.installs);
    if !local.is_empty() {
      snapshot.local_agents.insert(skill_key(id), local);
    }
  }

  for (id, record) in &instruction_lock.instructions {
    let file = env.instruction_file(id);
    let Ok(meta) = fs::metadata(&file) else {
      snapshot.skipped.push(record.name.clone());
      continue;
    };
    if meta.len() > MAX_FILE_BYTES {
      snapshot.skipped.push(record.name.clone());
      continue;
    }
    let content = fs::read(&file).map_err(|e| e.to_string())?;
    snapshot.contents.insert(
      format!("{}/{}.md", INSTRUCTIONS_DIR, id),
      (MODE_FILE.to_string(), content),
    );
    snapshot.library.instructions.insert(
      id.clone(),
      LibraryInstruction {
        name: record.name.clone(),
        source: shared_instruction_source(&record.source),
        hash: record.hash.clone(),
        imported_at: record.imported_at.clone(),
        updated_at: record.updated_at.clone(),
        scope: local_scope(
          base
            .and_then(|library| library.instructions.get(id))
            .map(|instruction| &instruction.scope),
          device,
        ),
      },
    );
    let local = user_agents(&record.installs);
    if !local.is_empty() {
      snapshot.local_agents.insert(instruction_key(id), local);
    }
  }

  let base_agents = state.base_agents.as_deref().unwrap_or_default();
  snapshot.agents = read_user_agent_apps(env)?
    .into_iter()
    .map(|app| {
      let scope = local_scope(
        base_agents
          .iter()
          .find(|agent| agent.app.id == app.id)
          .map(|agent| &agent.scope),
        device,
      );
      SyncedAgent { app, scope }
    })
    .collect();

  for (key, version) in &state.deferred {
    snapshot.carry(key, Some(version));
  }
  for (key, incoming) in &state.incoming {
    snapshot.carry(key, incoming.remote.as_ref());
  }

  // The device file: the base device file plus the install changes here since the base,
  // for the items this device has.
  let base_device = state.base_device.as_ref().map(DeviceFile::by_item);
  let keys: BTreeSet<String> = base_device
    .iter()
    .flat_map(|items| items.keys().cloned())
    .chain(snapshot.local_agents.keys().cloned())
    .collect();
  let mut installs = BTreeMap::new();
  for key in keys {
    let present = match split_key(&key) {
      Some((ItemKind::Skill, id)) => snapshot.library.skills.contains_key(id),
      Some((ItemKind::Instruction, id)) => snapshot.library.instructions.contains_key(id),
      _ => false,
    };
    if !present {
      continue;
    }
    let base_agents = base_device
      .as_ref()
      .map(|items| items.get(&key).cloned().unwrap_or_default());
    let empty = Vec::new();
    let agents = effective_agents(
      base_agents.as_ref(),
      state.base_local_agents.get(&key),
      snapshot.local_agents.get(&key).unwrap_or(&empty),
    );
    if !agents.is_empty() {
      installs.insert(key, agents);
    }
  }
  snapshot.device = DeviceFile {
    name: state.device_label(),
    installs: DeviceFile::installs_from(&installs),
    last_backup_at: state
      .base_device
      .as_ref()
      .and_then(|device| device.last_backup_at.clone()),
  };
  Ok(snapshot)
}

/// A full repository tree: this device's snapshot and the parts of other devices.
struct Composed {
  tree: BTreeMap<String, TreeFile>,
  /// Content of every tree file written from this computer.
  contents: BTreeMap<String, (String, Vec<u8>)>,
  library: LibraryFile,
  agents: Vec<SyncedAgent>,
  device: DeviceFile,
}

/// The tree a backup writes. `backup_at` sets the backup time of the device file.
fn compose(
  snapshot: &Snapshot,
  foreign: &Foreign,
  device: &str,
  backup_at: Option<String>,
) -> Result<Composed, String> {
  let mut library = snapshot.library.clone();
  library.version = LIBRARY_VERSION;
  for (id, skill) in &foreign.skills {
    library
      .skills
      .entry(id.clone())
      .or_insert_with(|| skill.clone());
  }
  for (id, instruction) in &foreign.instructions {
    library
      .instructions
      .entry(id.clone())
      .or_insert_with(|| instruction.clone());
  }
  let mut agents = snapshot.agents.clone();
  for agent in &foreign.agents {
    if !agents.iter().any(|own| own.app.id == agent.app.id) {
      agents.push(agent.clone());
    }
  }
  agents.sort_by(|a, b| a.app.id.cmp(&b.app.id));
  let mut device_file = snapshot.device.clone();
  if backup_at.is_some() {
    device_file.last_backup_at = backup_at;
  }

  let mut contents = snapshot.contents.clone();
  let file = |content: Vec<u8>| (MODE_FILE.to_string(), content);
  contents.insert(LIBRARY_FILE.to_string(), file(pretty_json(&library)?));
  contents.insert(AGENTS_FILE.to_string(), file(pretty_json(&agents)?));
  contents.insert(README_FILE.to_string(), file(readme(&library)));
  contents.insert(GITATTRIBUTES_FILE.to_string(), file(b"* -text\n".to_vec()));
  if !device.is_empty() {
    contents.insert(device_path(device), file(pretty_json(&device_file)?));
  }

  let mut tree = foreign.files.clone();
  tree.extend(snapshot.carried.clone());
  tree.extend(contents.iter().map(|(path, (mode, content))| {
    (
      path.clone(),
      TreeFile {
        mode: mode.clone(),
        sha: blob_sha(content),
      },
    )
  }));
  Ok(Composed {
    tree,
    contents,
    library,
    agents,
    device: device_file,
  })
}

/// Items that differ from `base` to `next`.
fn diff_items(
  base: &BTreeMap<String, ItemVersion>,
  next: &BTreeMap<String, ItemVersion>,
) -> Vec<LibraryChange> {
  let keys: BTreeSet<&String> = base.keys().chain(next.keys()).collect();
  let mut changes = Vec::new();
  for key in keys {
    let (before, after) = (base.get(key), next.get(key));
    let change = match (before, after) {
      (None, Some(_)) => ChangeKind::Added,
      (Some(_), None) => ChangeKind::Removed,
      (Some(before), Some(after)) if before != after => ChangeKind::Modified,
      _ => continue,
    };
    let (Some(version), Some((kind, id))) = (after.or(before), split_key(key)) else {
      continue;
    };
    changes.push(LibraryChange {
      kind,
      id: id.to_string(),
      name: version.name(),
      change,
    });
  }
  changes.sort_by_key(|change| (change.kind, change.name.to_lowercase()));
  changes
}

/// This device's changes since `base`: its items and its device file.
fn local_changes(base: &RepoState, snapshot: &Snapshot, device: &str) -> Vec<LibraryChange> {
  let mut changes = diff_items(&base.items(device), &snapshot.items());
  let change = match &base.device {
    // Before the first sync an empty device file is not a change of its own.
    None if base.files.is_empty() && snapshot.device.installs.is_empty() => None,
    None => Some(ChangeKind::Added),
    Some(before) if !before.same_installs(&snapshot.device) => Some(ChangeKind::Modified),
    Some(_) => None,
  };
  if let Some(change) = change {
    changes.push(LibraryChange {
      kind: ItemKind::Device,
      id: device.to_string(),
      name: snapshot.device.name.clone(),
      change,
    });
  }
  changes
}

fn library_is_empty(env: &Env) -> Result<bool, String> {
  Ok(
    store(env).read()?.skills.is_empty()
      && instruction_service::read_lock(env)?.instructions.is_empty(),
  )
}

/// Run the listing migrations (ids, untracked hub directories, named instruction records)
/// before taking the operation lock: they take it themselves.
fn normalize_library(env: &Env) -> Result<(), String> {
  hub_service::list_hub_skills(env)?;
  instruction_service::list_instructions(env)?;
  Ok(())
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Offline: connection and what changed on this computer since the last sync.
pub fn status(env: &Env) -> Result<SyncStatus, String> {
  let state = read_state(env)?;
  let library_empty = library_is_empty(env)?;
  if state.connected().is_err() {
    return Ok(SyncStatus {
      connected: false,
      repo: None,
      branch: None,
      device_name: None,
      last_backup_at: None,
      last_pull_at: None,
      has_synced: false,
      legacy_base: false,
      changes: Vec::new(),
      skipped: Vec::new(),
      library_empty,
      checkpoint: None,
    });
  }
  let device = state.device();
  let snapshot = local_snapshot(env, &state, &device)?;
  let mut base = state.base();
  if state.base_agents.is_none() && state.last_synced_commit.is_some() {
    // A base from before version 2: its custom agents are read on the next sync.
    base.agents = snapshot.agents.clone();
  }
  Ok(SyncStatus {
    connected: true,
    repo: state.repo.clone(),
    branch: state.branch.clone(),
    device_name: state.device_name.clone(),
    last_backup_at: state.last_backup_at.clone(),
    last_pull_at: state.last_pull_at.clone(),
    has_synced: state.last_synced_commit.is_some(),
    legacy_base: state.last_synced_commit.is_some() && base.library.version < LIBRARY_VERSION,
    changes: local_changes(&base, &snapshot, &device),
    skipped: snapshot.skipped,
    library_empty,
    checkpoint: checkpoint::info(env),
  })
}

// ---------------------------------------------------------------------------
// Connect
// ---------------------------------------------------------------------------

/// Name of this computer, shown in the repository history and its device file.
pub fn default_device_name() -> String {
  let from_command = |program: &str, args: &[&str]| -> Option<String> {
    let output = std::process::Command::new(program)
      .args(args)
      .output()
      .ok()?;
    if !output.status.success() {
      return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
  };
  if cfg!(target_os = "macos") {
    if let Some(name) = from_command("scutil", &["--get", "ComputerName"]) {
      return name;
    }
  }
  if let Ok(name) = std::env::var("COMPUTERNAME") {
    if !name.trim().is_empty() {
      return name.trim().to_string();
    }
  }
  from_command("hostname", &[]).unwrap_or_else(|| "this computer".to_string())
}

/// Record the repository to sync with. Keeps the sync base when it is the same repository.
/// This computer gets a device id when it has none.
pub fn set_connection(
  env: &Env,
  repo: &str,
  branch: &str,
  device_name: &str,
) -> Result<SyncStatus, String> {
  {
    let _ops = ops_guard(env)?;
    let mut state = read_state(env)?;
    let same_repo = state
      .repo
      .as_deref()
      .is_some_and(|current| current.eq_ignore_ascii_case(repo))
      && state.branch.as_deref() == Some(branch);
    if !same_repo {
      state.reset_base();
    }
    state.repo = Some(repo.to_string());
    state.branch = Some(branch.to_string());
    state.device_name = Some(device_name.to_string());
    if state.device_id.is_none() {
      state.device_id = Some(uuid::Uuid::new_v4().to_string());
    }
    write_state(env, &state)?;
  }
  status(env)
}

/// Connect to `name` (`repo` or `owner/repo`) with the account selected in Settings >
/// GitHub. A missing repository under that account is created as a private repository;
/// a public one is refused.
pub fn connect(env: &Env, name: &str) -> Result<SyncStatus, String> {
  let name = name.trim().trim_matches('/');
  let name = if name.is_empty() {
    DEFAULT_REPO_NAME
  } else {
    name
  };
  let token = git_remote::github_token()?;
  let login = git_remote::current_login(&token)?;
  let full_name = if name.contains('/') {
    name.to_string()
  } else {
    format!("{}/{}", login, name)
  };
  let (owner, repo_name) = full_name
    .split_once('/')
    .ok_or_else(|| format!("Invalid repository name: {}", full_name))?;
  if repo_name.is_empty() || repo_name.contains('/') {
    return Err(format!("Invalid repository name: {}", full_name));
  }

  let info = match git_remote::repo_info(&token, &full_name)? {
    Some(info) => info,
    None if owner.eq_ignore_ascii_case(&login) => {
      git_remote::create_private_repo(&token, repo_name)?
    },
    None => {
      return Err(format!(
        "Repository {} was not found or the account {} cannot see it",
        full_name, login
      ))
    },
  };
  if !info.private {
    return Err(format!(
      "{} is public. Choose a private repository for your library",
      info.full_name
    ));
  }
  if !info.can_push {
    return Err(format!(
      "The account {} cannot write to {}",
      login, info.full_name
    ));
  }
  let remote = GithubRepo::new(token, info.full_name.clone(), info.default_branch.clone())?;
  if remote.head()?.is_none() {
    remote.initialize()?;
  }
  let device_name = read_state(env)?
    .device_name
    .filter(|name| !name.trim().is_empty())
    .unwrap_or_else(default_device_name);
  set_connection(env, &info.full_name, &info.default_branch, &device_name)
}

/// Forget the repository on this computer. The repository and the library are untouched.
pub fn disconnect(env: &Env) -> Result<(), String> {
  let _ops = ops_guard(env)?;
  let path = state_path(env);
  if path.exists() {
    fs::remove_file(&path).map_err(|e| e.to_string())?;
  }
  let staging = env.youskill_root.join(STAGING_DIR);
  if staging.exists() {
    let _ = remove_path_any(&staging);
  }
  checkpoint::discard(env);
  Ok(())
}

fn github_remote(env: &Env) -> Result<GithubRepo, String> {
  let (repo, branch) = read_state(env)?.connected()?;
  GithubRepo::new(git_remote::github_token()?, repo, branch)
}

// ---------------------------------------------------------------------------
// Backup
// ---------------------------------------------------------------------------

pub fn backup(env: &Env) -> Result<BackupOutcome, String> {
  let remote = github_remote(env)?;
  backup_with(env, &remote)
}

fn commit_message(device: &str, changes: &[LibraryChange]) -> String {
  let count = |kind: ChangeKind| changes.iter().filter(|c| c.change == kind).count();
  format!(
    "Backup from {}: {} added, {} changed, {} removed",
    device,
    count(ChangeKind::Added),
    count(ChangeKind::Modified),
    count(ChangeKind::Removed)
  )
}

/// Upload every file of `tree` whose blob is not in one of the `known` trees.
fn upload_blobs(
  remote: &dyn RemoteRepo,
  contents: &BTreeMap<String, (String, Vec<u8>)>,
  tree: &BTreeMap<String, TreeFile>,
  known: &[&BTreeMap<String, TreeFile>],
) -> Result<(), String> {
  let known: BTreeSet<&String> = known
    .iter()
    .flat_map(|files| files.values().map(|file| &file.sha))
    .collect();
  for (path, file) in tree {
    if known.contains(&file.sha) {
      continue;
    }
    let Some((_, content)) = contents.get(path) else {
      return Err(format!("The repository no longer has {}", path));
    };
    let sha = remote.create_blob(content)?;
    if sha != file.sha {
      return Err(format!("GitHub stored {} with an unexpected id", path));
    }
  }
  Ok(())
}

/// Record a commit of `composed` as the new base.
fn set_base(state: &mut SyncState, commit: String, composed: Composed, snapshot: &Snapshot) {
  state.last_synced_commit = Some(commit);
  state.base_files = composed.tree;
  state.base_library = Some(composed.library);
  state.base_agents = Some(composed.agents);
  state.base_device = Some(composed.device);
  state.base_local_agents = snapshot.local_agents.clone();
}

/// Back up this device's items and device file. When another device moved the branch
/// since the base, each item is compared again against the head: without a conflict the
/// backup commits on top of the head and keeps the head-only changes for the next restore;
/// with one it returns `diverged` and writes nothing.
pub fn backup_with(env: &Env, remote: &dyn RemoteRepo) -> Result<BackupOutcome, String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  let device = ensure_device(env, &mut state)?;
  fill_base_agents(env, remote, &mut state)?;
  let mut snapshot = local_snapshot(env, &state, &device)?;
  let base = state.base();
  let changes = local_changes(&base, &snapshot, &device);
  let skipped = snapshot.skipped.clone();
  let outcome =
    |result: BackupResult, commit: Option<String>, changes: Vec<LibraryChange>| BackupOutcome {
      result,
      commit,
      changes,
      skipped: skipped.clone(),
      incoming: 0,
    };

  let head = remote.head()?;
  let head_files = match &head {
    Some(head) => remote.tree(&head.tree)?,
    None => BTreeMap::new(),
  };
  let head_commit = head.as_ref().map(|head| head.commit.clone());
  let mut incoming = 0;
  let head_state = match state.last_synced_commit.clone() {
    None => {
      if head_files.contains_key(LIBRARY_FILE) {
        return Ok(outcome(BackupResult::RemoteNotEmpty, None, changes));
      }
      RepoState::default()
    },
    Some(base_commit) if head_commit.as_deref() == Some(base_commit.as_str()) => {
      if base.library.version < LIBRARY_VERSION {
        return Ok(outcome(BackupResult::MigrationRequired, None, changes));
      }
      // Without changes a backup still writes a `library.json` that holds local paths of
      // an earlier version: the library read from it has none, so the files differ.
      if changes.is_empty() {
        let composed = compose(&snapshot, &base.foreign(&device), &device, None)?;
        if composed.tree.get(LIBRARY_FILE) == base.files.get(LIBRARY_FILE) {
          return Ok(outcome(BackupResult::UpToDate, Some(base_commit), changes));
        }
      }
      base.clone()
    },
    Some(_) => {
      let loaded = match &head_commit {
        Some(commit) => load_repo_state(remote, commit, &head_files, &device)?,
        None => None,
      };
      let result = if changes.is_empty() {
        BackupResult::RemoteAhead
      } else {
        BackupResult::Diverged
      };
      let Some(loaded) = loaded else {
        return Ok(outcome(result, None, changes));
      };
      if loaded.library.version < LIBRARY_VERSION {
        return Ok(outcome(BackupResult::MigrationRequired, None, changes));
      }
      if changes.is_empty() {
        return Ok(outcome(BackupResult::RemoteAhead, None, changes));
      }
      match rebase(&state, &base, &snapshot, &loaded, &device) {
        Some(next) => {
          incoming = next.incoming.len();
          state.incoming = next.incoming;
          state.deferred = next.deferred;
          snapshot = local_snapshot(env, &state, &device)?;
        },
        None => return Ok(outcome(BackupResult::Diverged, None, changes)),
      }
      loaded
    },
  };

  let mut composed = compose(&snapshot, &head_state.foreign(&device), &device, None)?;
  if composed.tree == head_files {
    // The head already holds this library: it becomes the base.
    if let Some(commit) = head_commit.clone() {
      set_base(&mut state, commit, composed, &snapshot);
      write_state(env, &state)?;
    }
    let mut done = outcome(BackupResult::UpToDate, head_commit, changes);
    done.incoming = incoming;
    return Ok(done);
  }
  let now = now_rfc3339();
  composed = compose(
    &snapshot,
    &head_state.foreign(&device),
    &device,
    Some(now.clone()),
  )?;
  upload_blobs(
    remote,
    &composed.contents,
    &composed.tree,
    &[&head_files, &state.base_files],
  )?;
  let tree_sha = remote.create_tree(&composed.tree)?;
  let commit = remote.create_commit(
    &commit_message(&state.device_label(), &changes),
    &tree_sha,
    head_commit.as_deref(),
  )?;
  match remote.update_head(&commit) {
    Ok(()) => {},
    Err(UpdateError::Rejected) => return Ok(outcome(BackupResult::RemoteAhead, None, changes)),
    Err(UpdateError::Other(err)) => return Err(err),
  }

  state.last_backup_at = Some(now);
  set_base(&mut state, commit.clone(), composed, &snapshot);
  write_state(env, &state)?;
  let mut done = outcome(BackupResult::Pushed, Some(commit), changes);
  done.incoming = incoming;
  Ok(done)
}

/// The carried items after a backup on a moved head.
struct Rebased {
  incoming: BTreeMap<String, IncomingItem>,
  deferred: BTreeMap<String, ItemVersion>,
}

/// Compare this device's items in the base, the snapshot and the head. `None` when an item
/// changed on both sides; otherwise the head-only changes join the incoming items (a
/// deferred item the head changed again becomes incoming).
fn rebase(
  state: &SyncState,
  base: &RepoState,
  snapshot: &Snapshot,
  head: &RepoState,
  device: &str,
) -> Option<Rebased> {
  let base_items = base.items(device);
  let local_items = snapshot.items();
  let head_items = head.items(device);
  let mut next = Rebased {
    incoming: state.incoming.clone(),
    deferred: state.deferred.clone(),
  };
  let keys: BTreeSet<&String> = base_items
    .keys()
    .chain(local_items.keys())
    .chain(head_items.keys())
    .collect();
  for key in keys {
    let (b, l, h) = (
      base_items.get(key),
      local_items.get(key),
      head_items.get(key),
    );
    match restore::resolve(restore::change(b, l), restore::change(b, h), l, h) {
      Some(restore::Resolution::Conflict) => return None,
      Some(restore::Resolution::TakeRemote) | Some(restore::Resolution::RemovalCandidate) => {
        let item = next
          .incoming
          .entry(key.clone())
          .or_insert_with(|| IncomingItem {
            base: b.cloned(),
            remote: None,
          });
        item.remote = h.cloned();
        next.deferred.remove(key);
      },
      _ => {},
    }
  }
  Some(next)
}

// ---------------------------------------------------------------------------
// Pending installs
// ---------------------------------------------------------------------------

/// User-level install changes that bring this computer in line with its device file:
/// installs it lists for agents present here but not installed, installs here it does not
/// list, and copy installs behind the hub. The installs of other devices never show here.
pub fn pending(env: &Env) -> Result<Vec<PendingItem>, String> {
  let state = read_state(env)?;
  if state.last_synced_commit.is_none() {
    return Ok(Vec::new());
  }
  let snapshot = local_snapshot(env, &state, &state.device())?;
  let wanted = snapshot.device.by_item();
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  let mut items = Vec::new();
  let empty = Vec::new();

  for (id, skill) in &snapshot.library.skills {
    let Some(record) = lock.skills.get(id) else {
      continue;
    };
    let local = user_agents(&record.installs);
    let enabled = wanted.get(&skill_key(id)).unwrap_or(&empty);
    for agent in enabled {
      let Some(app) = env.agent(agent) else {
        continue;
      };
      if app.global_path.is_some() && !local.contains(agent) {
        items.push(PendingItem {
          kind: ItemKind::Skill,
          id: id.clone(),
          name: skill.name.clone(),
          action: PendingAction::Install,
          agent_id: agent.clone(),
          path: None,
        });
      }
    }
    for agent in &local {
      if !enabled.contains(agent) && env.agent(agent).is_some() {
        items.push(PendingItem {
          kind: ItemKind::Skill,
          id: id.clone(),
          name: skill.name.clone(),
          action: PendingAction::Uninstall,
          agent_id: agent.clone(),
          path: None,
        });
      }
    }
    for install in build_view(env, id, record).installs {
      if install.record.scope == InstallScope::User && install.state == TargetState::Outdated {
        items.push(PendingItem {
          kind: ItemKind::Skill,
          id: id.clone(),
          name: skill.name.clone(),
          action: PendingAction::Update,
          agent_id: install
            .record
            .agent_ids
            .first()
            .cloned()
            .unwrap_or_default(),
          path: Some(install.record.path.clone()),
        });
      }
    }
  }

  for (id, instruction) in &snapshot.library.instructions {
    let Some(record) = instruction_lock.instructions.get(id) else {
      continue;
    };
    let local = user_agents(&record.installs);
    let enabled = wanted.get(&instruction_key(id)).unwrap_or(&empty);
    for agent in enabled {
      let Some(app) = env.agent(agent) else {
        continue;
      };
      if app.global_profile_path.is_some() && !local.contains(agent) {
        items.push(PendingItem {
          kind: ItemKind::Instruction,
          id: id.clone(),
          name: instruction.name.clone(),
          action: PendingAction::Install,
          agent_id: agent.clone(),
          path: None,
        });
      }
    }
    for agent in &local {
      if !enabled.contains(agent) && env.agent(agent).is_some() {
        items.push(PendingItem {
          kind: ItemKind::Instruction,
          id: id.clone(),
          name: instruction.name.clone(),
          action: PendingAction::Uninstall,
          agent_id: agent.clone(),
          path: None,
        });
      }
    }
    for install in instruction_service::build_view(env, id, record).installs {
      if install.record.scope == InstallScope::User && install.state == TargetState::Outdated {
        items.push(PendingItem {
          kind: ItemKind::Instruction,
          id: id.clone(),
          name: instruction.name.clone(),
          action: PendingAction::Update,
          agent_id: install
            .record
            .agent_ids
            .first()
            .cloned()
            .unwrap_or_default(),
          path: Some(install.record.path.clone()),
        });
      }
    }
  }

  items.sort_by(|a, b| {
    (a.action, a.kind, a.name.to_lowercase(), &a.agent_id).cmp(&(
      b.action,
      b.kind,
      b.name.to_lowercase(),
      &b.agent_id,
    ))
  });
  Ok(items)
}

/// Run the selected pending items with the existing install, uninstall and push actions.
/// Nothing is forced: an item those actions refuse is reported, not overwritten.
pub fn apply_pending(
  env: &Env,
  items: &[PendingItem],
  mode: InstallMode,
) -> Result<ApplyPendingOutcome, String> {
  let mut groups: BTreeMap<(ItemKind, String, PendingAction), Vec<&PendingItem>> = BTreeMap::new();
  for item in items {
    groups
      .entry((item.kind, item.id.clone(), item.action))
      .or_default()
      .push(item);
  }

  let mut outcome = ApplyPendingOutcome::default();
  for ((kind, id, action), group) in groups {
    let name = group
      .first()
      .map(|item| item.name.clone())
      .unwrap_or_default();
    let targets: Vec<InstallTargetSpec> = group
      .iter()
      .map(|item| InstallTargetSpec {
        scope: InstallScope::User,
        project_path: None,
        agent_id: item.agent_id.clone(),
      })
      .collect();
    let paths: Vec<String> = group.iter().filter_map(|item| item.path.clone()).collect();
    let result: Result<(bool, Vec<String>), String> = match (kind, action) {
      (ItemKind::Skill, PendingAction::Install) => install_service::install_skill(
        env,
        InstallRequest {
          name: id.clone(),
          targets,
          mode: Some(mode),
          force: false,
        },
      )
      .map(|r| (r.applied, r.blockers)),
      (ItemKind::Skill, PendingAction::Uninstall) => install_service::uninstall_skill(
        env,
        UninstallRequest {
          name: id.clone(),
          targets,
          paths: Vec::new(),
          force: false,
        },
      )
      .map(|r| (r.applied, r.blockers)),
      (ItemKind::Skill, PendingAction::Update) => hub_service::sync_skill(
        env,
        &id,
        SyncAction::PushTargets {
          targets: Some(paths),
          force: false,
        },
      )
      .map(|r| (r.applied, r.blockers)),
      (ItemKind::Instruction, PendingAction::Install) => instruction_service::install_instruction(
        env,
        InstallRequest {
          name: id.clone(),
          targets,
          mode: Some(mode),
          force: false,
        },
      )
      .map(|r| (r.applied, r.blockers)),
      (ItemKind::Instruction, PendingAction::Uninstall) => {
        instruction_service::uninstall_instruction(
          env,
          UninstallRequest {
            name: id.clone(),
            targets,
            paths: Vec::new(),
            force: false,
          },
        )
        .map(|r| (r.applied, r.blockers))
      },
      (ItemKind::Instruction, PendingAction::Update) => instruction_service::sync_instruction(
        env,
        &id,
        SyncAction::PushTargets {
          targets: Some(paths),
          force: false,
        },
      )
      .map(|r| (r.applied, r.blockers)),
      (ItemKind::Agent | ItemKind::Device, _) => continue,
    };
    match result {
      Ok((true, _)) => outcome.applied += group.len(),
      Ok((false, blockers)) => {
        outcome
          .failures
          .extend(blockers.into_iter().map(|reason| PendingFailure {
            kind,
            name: name.clone(),
            reason,
          }))
      },
      Err(reason) => outcome.failures.push(PendingFailure { kind, name, reason }),
    }
  }
  Ok(outcome)
}

#[cfg(test)]
mod tests;
