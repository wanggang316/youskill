//! Back up the library to a private GitHub repository and bring it to other computers.
//!
//! The repository holds what is the same on every computer: skill and instruction files,
//! their shared metadata (`library.json`), the agents a skill is enabled for at user level
//! (by agent id, never by path) and the custom agent definitions (`agents.json`). Install
//! paths, project installs, settings and secrets stay on each computer.
//!
//! The local lock files stay the source of truth. A backup exports a snapshot of them and
//! commits it on top of the branch; it only moves forward from the last synced commit. A
//! restore (`restore`) merges the branch, or an earlier commit of it, into the hub item by
//! item and commits the merged result on top of the branch. Repository history is never
//! rewritten.

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
mod restore;

pub use crate::utils::git_remote::RemoteCommit;
pub use checkpoint::{undo_restore, CheckpointInfo};
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
const USER_AGENT_APPS_FILE: &str = "user_agent_apps.json";
const LIBRARY_FORMAT: &str = "youskill-library";
const LIBRARY_VERSION: u32 = 1;
/// GitHub refuses files over 100 MB; a skill with a file over this stays on this computer.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Shared metadata (`library.json`)
// ---------------------------------------------------------------------------

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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySkill {
  pub name: String,
  #[serde(default)]
  pub source: SkillSource,
  pub hash: String,
  pub imported_at: String,
  pub updated_at: String,
  /// Agents the skill is enabled for at user level.
  #[serde(default)]
  pub agents: Vec<String>,
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
  pub agents: Vec<String>,
}

// ---------------------------------------------------------------------------
// Local state
// ---------------------------------------------------------------------------

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
  /// User-level agents installed on this computer at the base, by `skill:<id>` /
  /// `instruction:<id>`. Lets a backup tell this computer's install changes apart from
  /// agents it never had.
  #[serde(default)]
  pub base_local_agents: BTreeMap<String, Vec<String>>,
  /// Remote items the user left out of a restore, by item key, with the version they had.
  /// A backup carries this version forward instead of the local one.
  #[serde(default)]
  pub deferred: BTreeMap<String, ItemVersion>,
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
    self.base_local_agents.clear();
    self.deferred.clear();
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
  serde_json::from_str(&content).map_err(|e| format!("Failed to parse {}: {}", path.display(), e))
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
  /// The custom agent definitions.
  Agents,
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
  /// Both changed since the last sync.
  Diverged,
  /// The repository already holds a library this computer never synced with.
  RemoteNotEmpty,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackupOutcome {
  pub result: BackupResult,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub commit: Option<String>,
  pub changes: Vec<LibraryChange>,
  pub skipped: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PendingAction {
  /// Enabled in the library, not installed for this agent here.
  Install,
  /// Installed here, no longer enabled in the library.
  Uninstall,
  /// A copy install behind the hub.
  Update,
}

/// An install change that would bring this computer in line with the library.
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
  pub failures: Vec<String>,
}

// ---------------------------------------------------------------------------
// Snapshot of this computer's library
// ---------------------------------------------------------------------------

struct Snapshot {
  /// Repository path -> (mode, content).
  contents: BTreeMap<String, (String, Vec<u8>)>,
  /// Files carried forward from the repository without local content (deferred items).
  carried: BTreeMap<String, TreeFile>,
  library: LibraryFile,
  local_agents: BTreeMap<String, Vec<String>>,
  skipped: Vec<String>,
}

impl Snapshot {
  fn tree(&self) -> BTreeMap<String, TreeFile> {
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

  /// Replace the local version of each deferred item with the repository version it had,
  /// so a deferred item is neither a local change nor removed by the next backup.
  fn carry_deferred(&mut self, deferred: &BTreeMap<String, ItemVersion>) {
    for (key, version) in deferred {
      let Some((kind, id)) = split_key(key) else {
        continue;
      };
      let prefix = match kind {
        ItemKind::Skill => {
          self.library.skills.remove(id);
          format!("{}/{}/", SKILLS_DIR, id)
        },
        ItemKind::Instruction => {
          self.library.instructions.remove(id);
          format!("{}/{}.md", INSTRUCTIONS_DIR, id)
        },
        ItemKind::Agents => AGENTS_FILE.to_string(),
      };
      self.contents.retain(|path, _| !path.starts_with(&prefix));
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
        Entry::Agents => {},
      }
      self.carried.extend(version.paths(id));
    }
  }
}

// ---------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------

const AGENTS_KEY: &str = "agents";

/// The library entry of an item, or the custom agents file, which has none.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
  Skill(LibrarySkill),
  Instruction(LibraryInstruction),
  Agents,
}

/// One item in one state of the library: its entry and its files, by path below the item
/// (inside `skills/<id>/<name>/`; empty for a one-file item).
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
      Entry::Agents => ItemKind::Agents,
    }
  }

  fn name(&self) -> String {
    match &self.entry {
      Entry::Skill(skill) => skill.name.clone(),
      Entry::Instruction(instruction) => instruction.name.clone(),
      Entry::Agents => String::new(),
    }
  }

  /// The entry without the user-level agents, which merge per agent and never conflict.
  fn shared_entry(&self) -> Entry {
    match &self.entry {
      Entry::Skill(skill) => Entry::Skill(LibrarySkill {
        agents: Vec::new(),
        ..skill.clone()
      }),
      Entry::Instruction(instruction) => Entry::Instruction(LibraryInstruction {
        agents: Vec::new(),
        ..instruction.clone()
      }),
      Entry::Agents => Entry::Agents,
    }
  }

  fn same_content(&self, other: &ItemVersion) -> bool {
    self.files == other.files && self.shared_entry() == other.shared_entry()
  }

  /// The source key that pairs items on a first merge: a GitHub or folder source.
  fn source_key(&self) -> Option<String> {
    match &self.entry {
      Entry::Skill(skill) => match &skill.source {
        SkillSource::Github { .. } | SkillSource::Folder { .. } => Some(skill.source.key()),
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
      Entry::Agents => None,
    }
  }

  /// Repository paths of the item's files.
  fn paths(&self, id: &str) -> BTreeMap<String, TreeFile> {
    self
      .files
      .iter()
      .map(|(relative, file)| {
        let path = match &self.entry {
          Entry::Skill(skill) => format!("{}/{}/{}/{}", SKILLS_DIR, id, skill.name, relative),
          Entry::Instruction(_) => format!("{}/{}.md", INSTRUCTIONS_DIR, id),
          Entry::Agents => AGENTS_FILE.to_string(),
        };
        (path, file.clone())
      })
      .collect()
  }
}

fn split_key(key: &str) -> Option<(ItemKind, &str)> {
  if key == AGENTS_KEY {
    return Some((ItemKind::Agents, AGENTS_FILE));
  }
  if let Some(id) = key.strip_prefix("skill:") {
    return Some((ItemKind::Skill, id));
  }
  key
    .strip_prefix("instruction:")
    .map(|id| (ItemKind::Instruction, id))
}

fn empty_agents_file() -> TreeFile {
  TreeFile {
    mode: MODE_FILE.to_string(),
    sha: blob_sha(&agents_json(&[]).unwrap_or_default()),
  }
}

/// Every item of one state of the library, by item key. The custom agents file is always
/// an item; a missing file counts as an empty list.
fn item_versions(
  library: &LibraryFile,
  files: &BTreeMap<String, TreeFile>,
) -> BTreeMap<String, ItemVersion> {
  let mut items = BTreeMap::new();
  for (id, skill) in &library.skills {
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
  let agents = files
    .get(AGENTS_FILE)
    .cloned()
    .unwrap_or_else(empty_agents_file);
  items.insert(
    AGENTS_KEY.to_string(),
    ItemVersion {
      entry: Entry::Agents,
      files: BTreeMap::from([(String::new(), agents)]),
    },
  );
  items
}

fn skill_key(id: &str) -> String {
  format!("skill:{}", id)
}

fn instruction_key(id: &str) -> String {
  format!("instruction:{}", id)
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

/// The library's agents after this computer's own install changes since the base: an agent
/// installed here is added, one uninstalled here is removed, and agents this computer
/// never had (not installed, maybe not even present) are kept as they were.
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

/// A GitHub source without the fields `check_source_updates` keeps refreshing.
fn shared_source(source: &SkillSource) -> SkillSource {
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

/// A pulled source, keeping this computer's last update check when the location is the same.
fn merged_source(local: &SkillSource, remote: &SkillSource) -> SkillSource {
  match (local, remote) {
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

fn agents_json(apps: &[AgentApp]) -> Result<Vec<u8>, String> {
  let mut text = serde_json::to_string_pretty(apps).map_err(|e| e.to_string())?;
  text.push('\n');
  Ok(text.into_bytes())
}

fn library_json(library: &LibraryFile) -> Result<Vec<u8>, String> {
  let mut text = serde_json::to_string_pretty(library).map_err(|e| e.to_string())?;
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
        table_cell(&skill.source.key())
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

fn local_snapshot(env: &Env, state: &SyncState) -> Result<Snapshot, String> {
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  let base = state.base_library.as_ref();
  let mut snapshot = Snapshot {
    contents: BTreeMap::new(),
    carried: BTreeMap::new(),
    library: LibraryFile::default(),
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
    let key = skill_key(id);
    let local = user_agents(&record.installs);
    let agents = effective_agents(
      base
        .and_then(|library| library.skills.get(id))
        .map(|skill| &skill.agents),
      state.base_local_agents.get(&key),
      &local,
    );
    snapshot.library.skills.insert(
      id.clone(),
      LibrarySkill {
        name: record.name.clone(),
        source: shared_source(&record.source),
        hash: record.hash.clone(),
        imported_at: record.imported_at.clone(),
        updated_at: record.updated_at.clone(),
        agents,
      },
    );
    if !local.is_empty() {
      snapshot.local_agents.insert(key, local);
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
    let key = instruction_key(id);
    let local = user_agents(&record.installs);
    let agents = effective_agents(
      base
        .and_then(|library| library.instructions.get(id))
        .map(|instruction| &instruction.agents),
      state.base_local_agents.get(&key),
      &local,
    );
    snapshot.library.instructions.insert(
      id.clone(),
      LibraryInstruction {
        name: record.name.clone(),
        source: record.source.clone(),
        hash: record.hash.clone(),
        imported_at: record.imported_at.clone(),
        updated_at: record.updated_at.clone(),
        agents,
      },
    );
    if !local.is_empty() {
      snapshot.local_agents.insert(key, local);
    }
  }

  snapshot.contents.insert(
    AGENTS_FILE.to_string(),
    (
      MODE_FILE.to_string(),
      agents_json(&read_user_agent_apps(env)?)?,
    ),
  );
  snapshot.carry_deferred(&state.deferred);
  snapshot.contents.insert(
    LIBRARY_FILE.to_string(),
    (MODE_FILE.to_string(), library_json(&snapshot.library)?),
  );
  snapshot.contents.insert(
    README_FILE.to_string(),
    (MODE_FILE.to_string(), readme(&snapshot.library)),
  );
  snapshot.contents.insert(
    GITATTRIBUTES_FILE.to_string(),
    (MODE_FILE.to_string(), b"* -text\n".to_vec()),
  );
  Ok(snapshot)
}

/// Items that differ from `base` to `next`. README and `.gitattributes` are derived and do
/// not count.
fn diff_library(
  base_library: Option<&LibraryFile>,
  base_files: &BTreeMap<String, TreeFile>,
  next_library: &LibraryFile,
  next_files: &BTreeMap<String, TreeFile>,
) -> Vec<LibraryChange> {
  let empty = LibraryFile::default();
  let base_library = base_library.unwrap_or(&empty);
  let files_under = |files: &BTreeMap<String, TreeFile>, prefix: &str| -> Vec<(String, TreeFile)> {
    files
      .range(prefix.to_string()..)
      .take_while(|(path, _)| path.starts_with(prefix))
      .map(|(path, file)| (path.clone(), file.clone()))
      .collect()
  };
  let mut changes = Vec::new();

  let skill_ids: BTreeSet<&String> = base_library
    .skills
    .keys()
    .chain(next_library.skills.keys())
    .collect();
  for id in skill_ids {
    let prefix = format!("{}/{}/", SKILLS_DIR, id);
    let change = match (base_library.skills.get(id), next_library.skills.get(id)) {
      (None, Some(_)) => Some(ChangeKind::Added),
      (Some(_), None) => Some(ChangeKind::Removed),
      (Some(before), Some(after)) => (before != after
        || files_under(base_files, &prefix) != files_under(next_files, &prefix))
      .then_some(ChangeKind::Modified),
      (None, None) => None,
    };
    if let Some(change) = change {
      let name = next_library
        .skills
        .get(id)
        .or_else(|| base_library.skills.get(id))
        .map(|skill| skill.name.clone())
        .unwrap_or_default();
      changes.push(LibraryChange {
        kind: ItemKind::Skill,
        id: id.clone(),
        name,
        change,
      });
    }
  }

  let instruction_ids: BTreeSet<&String> = base_library
    .instructions
    .keys()
    .chain(next_library.instructions.keys())
    .collect();
  for id in instruction_ids {
    let path = format!("{}/{}.md", INSTRUCTIONS_DIR, id);
    let change = match (
      base_library.instructions.get(id),
      next_library.instructions.get(id),
    ) {
      (None, Some(_)) => Some(ChangeKind::Added),
      (Some(_), None) => Some(ChangeKind::Removed),
      (Some(before), Some(after)) => (before != after
        || base_files.get(&path) != next_files.get(&path))
      .then_some(ChangeKind::Modified),
      (None, None) => None,
    };
    if let Some(change) = change {
      let name = next_library
        .instructions
        .get(id)
        .or_else(|| base_library.instructions.get(id))
        .map(|instruction| instruction.name.clone())
        .unwrap_or_default();
      changes.push(LibraryChange {
        kind: ItemKind::Instruction,
        id: id.clone(),
        name,
        change,
      });
    }
  }

  if base_files.get(AGENTS_FILE) != next_files.get(AGENTS_FILE) {
    let empty_list = agents_json(&[]).map(|bytes| blob_sha(&bytes)).ok();
    let has = |files: &BTreeMap<String, TreeFile>| {
      files
        .get(AGENTS_FILE)
        .is_some_and(|file| Some(&file.sha) != empty_list.as_ref())
    };
    let change = match (has(base_files), has(next_files)) {
      (false, true) => Some(ChangeKind::Added),
      (true, false) => Some(ChangeKind::Removed),
      (true, true) => Some(ChangeKind::Modified),
      (false, false) => None,
    };
    if let Some(change) = change {
      changes.push(LibraryChange {
        kind: ItemKind::Agents,
        id: AGENTS_FILE.to_string(),
        name: String::new(),
        change,
      });
    }
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
      changes: Vec::new(),
      skipped: Vec::new(),
      library_empty,
      checkpoint: None,
    });
  }
  let snapshot = local_snapshot(env, &state)?;
  let changes = diff_library(
    state.base_library.as_ref(),
    &state.base_files,
    &snapshot.library,
    &snapshot.tree(),
  );
  Ok(SyncStatus {
    connected: true,
    repo: state.repo.clone(),
    branch: state.branch.clone(),
    device_name: state.device_name.clone(),
    last_backup_at: state.last_backup_at.clone(),
    last_pull_at: state.last_pull_at.clone(),
    has_synced: state.last_synced_commit.is_some(),
    changes,
    skipped: snapshot.skipped,
    library_empty,
    checkpoint: checkpoint::info(env),
  })
}

// ---------------------------------------------------------------------------
// Connect
// ---------------------------------------------------------------------------

/// Name of this computer, shown in the repository history.
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
  snapshot: &Snapshot,
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
    let Some((_, content)) = snapshot.contents.get(path) else {
      return Err(format!("The repository no longer has {}", path));
    };
    let sha = remote.create_blob(content)?;
    if sha != file.sha {
      return Err(format!("GitHub stored {} with an unexpected id", path));
    }
  }
  Ok(())
}

pub fn backup_with(env: &Env, remote: &dyn RemoteRepo) -> Result<BackupOutcome, String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  let snapshot = local_snapshot(env, &state)?;
  let tree = snapshot.tree();
  let changes = diff_library(
    state.base_library.as_ref(),
    &state.base_files,
    &snapshot.library,
    &tree,
  );
  let outcome =
    |result: BackupResult, commit: Option<String>, changes: Vec<LibraryChange>| BackupOutcome {
      result,
      commit,
      changes,
      skipped: snapshot.skipped.clone(),
    };

  let head = remote.head()?;
  let remote_files = match &head {
    Some(head) => remote.tree(&head.tree)?,
    None => BTreeMap::new(),
  };
  match state.last_synced_commit.as_deref() {
    None => {
      if remote_files.contains_key(LIBRARY_FILE) {
        return Ok(outcome(BackupResult::RemoteNotEmpty, None, changes));
      }
    },
    Some(base) => {
      if head.as_ref().map(|head| head.commit.as_str()) != Some(base) {
        let result = if changes.is_empty() {
          BackupResult::RemoteAhead
        } else {
          BackupResult::Diverged
        };
        return Ok(outcome(result, None, changes));
      }
      if changes.is_empty() {
        return Ok(outcome(
          BackupResult::UpToDate,
          Some(base.to_string()),
          changes,
        ));
      }
    },
  }

  upload_blobs(remote, &snapshot, &tree, &[&remote_files])?;
  let device = state
    .device_name
    .clone()
    .unwrap_or_else(default_device_name);
  let tree_sha = remote.create_tree(&tree)?;
  let commit = remote.create_commit(
    &commit_message(&device, &changes),
    &tree_sha,
    head.as_ref().map(|head| head.commit.as_str()),
  )?;
  match remote.update_head(&commit) {
    Ok(()) => {},
    Err(UpdateError::Rejected) => return Ok(outcome(BackupResult::RemoteAhead, None, changes)),
    Err(UpdateError::Other(err)) => return Err(err),
  }

  state.last_synced_commit = Some(commit.clone());
  state.last_backup_at = Some(now_rfc3339());
  state.base_files = tree;
  state.base_library = Some(snapshot.library.clone());
  state.base_local_agents = snapshot.local_agents.clone();
  write_state(env, &state)?;
  Ok(outcome(BackupResult::Pushed, Some(commit), changes))
}

// ---------------------------------------------------------------------------
// Pending installs
// ---------------------------------------------------------------------------

/// User-level install changes that bring this computer in line with the library: agents a
/// skill is enabled for that are present here but not installed, installs the library no
/// longer enables, and copy installs behind the hub. Agents not installed on this computer
/// are left out.
pub fn pending(env: &Env) -> Result<Vec<PendingItem>, String> {
  let state = read_state(env)?;
  if state.last_synced_commit.is_none() {
    return Ok(Vec::new());
  }
  let snapshot = local_snapshot(env, &state)?;
  let lock = store(env).read()?;
  let instruction_lock = instruction_service::read_lock(env)?;
  let mut items = Vec::new();

  for (id, skill) in &snapshot.library.skills {
    let Some(record) = lock.skills.get(id) else {
      continue;
    };
    let local = user_agents(&record.installs);
    for agent in &skill.agents {
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
      if !skill.agents.contains(agent) && env.agent(agent).is_some() {
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
    for agent in &instruction.agents {
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
      if !instruction.agents.contains(agent) && env.agent(agent).is_some() {
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
      (ItemKind::Agents, _) => continue,
    };
    match result {
      Ok((true, _)) => outcome.applied += group.len(),
      Ok((false, blockers)) => outcome
        .failures
        .extend(blockers.into_iter().map(|b| format!("{}: {}", name, b))),
      Err(err) => outcome.failures.push(format!("{}: {}", name, err)),
    }
  }
  Ok(outcome)
}

#[cfg(test)]
mod tests;
