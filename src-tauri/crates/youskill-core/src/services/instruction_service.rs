//! Instruction templates (`~/.youskill/instructions/<id>.md`) and the agent files
//! (`AGENTS.md`, `CLAUDE.md`, ...) they are installed to. Templates are keyed by a
//! generated id and carry a free-text display name; agent files are listed and edited in
//! place and only enter the library when promoted to a template. Install records and the
//! three-way drift share the skill types.

use crate::models::{
  AgentFileTemplate, AgentFileView, AgentRootMatch, DetectedInstruction, HubState, InstallMode,
  InstallRecord, InstallRequest, InstallScope, InstallTargetSpec, InstallView,
  InstructionActionResult, InstructionImportItem, InstructionImportOutcome, InstructionLockFile,
  InstructionRecord, InstructionSource, InstructionView, SkillDiff, SyncAction, TargetState,
  UninstallRequest, LOCK_VERSION,
};
use crate::services::diff_service::diff_files;
use crate::services::drift_service::compare_three_way;
use crate::services::env::Env;
use crate::services::install_service::ResolvedTarget;
use crate::services::lock_service::{self, ops_guard};
use crate::services::workspace_service::references_of;
use crate::utils::folder::create_temp_dir;
use crate::utils::github::GithubHelper;
use crate::utils::hash::hash_file;
use crate::utils::path::{
  expand_home_with, is_symlink, is_within, normalize_dir_path, path_to_string, remove_path_any,
  same_entry, same_path, symlink_points_to, symlink_target, symlink_target_string,
};
use crate::utils::time::{now_file_stamp, now_rfc3339};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;
use walkdir::WalkDir;

const DESCRIPTION_MAX_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// Lock file
// ---------------------------------------------------------------------------

static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn read_lock(env: &Env) -> Result<InstructionLockFile, String> {
  let path = &env.instruction_lock_path;
  if !path.exists() {
    return Ok(InstructionLockFile::default());
  }
  let content = fs::read_to_string(path)
    .map_err(|e| format!("Failed to read {}: {}", path.to_string_lossy(), e))?;
  if content.trim().is_empty() {
    return Ok(InstructionLockFile::default());
  }
  serde_json::from_str(&content)
    .map_err(|e| format!("Failed to parse {}: {}", path.to_string_lossy(), e))
}

fn get_record(env: &Env, name: &str) -> Result<InstructionRecord, String> {
  read_lock(env)?
    .instructions
    .get(name)
    .cloned()
    .ok_or_else(|| format!("Instruction '{}' is not in the library", name))
}

/// Read-modify-write under the operation lock and a process-wide mutex; written through a
/// temp file + rename.
pub(crate) fn update_lock<T>(
  env: &Env,
  f: impl FnOnce(&mut InstructionLockFile) -> Result<T, String>,
) -> Result<T, String> {
  let _ops = lock_service::ops_guard_unless_held(&env.youskill_root)?;
  let _guard = WRITE_LOCK
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner());
  let mut lock = read_lock(env)?;
  let result = f(&mut lock)?;
  lock.version = LOCK_VERSION;
  let path = &env.instruction_lock_path;
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  let content = serde_json::to_string_pretty(&lock).map_err(|e| e.to_string())?;
  let tmp = path.with_extension("json.tmp");
  fs::write(&tmp, content).map_err(|e| e.to_string())?;
  fs::rename(&tmp, path).map_err(|e| {
    let _ = fs::remove_file(&tmp);
    e.to_string()
  })?;
  Ok(result)
}

fn record_mut<'a>(
  lock: &'a mut InstructionLockFile,
  name: &str,
) -> Result<&'a mut InstructionRecord, String> {
  lock
    .instructions
    .get_mut(name)
    .ok_or_else(|| format!("Instruction '{}' is not in the library", name))
}

// ---------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------

pub fn resolve_target(env: &Env, spec: &InstallTargetSpec) -> Result<ResolvedTarget, String> {
  let app = env
    .agent(&spec.agent_id)
    .ok_or_else(|| format!("Unknown agent app: {}", spec.agent_id))?;
  let project_root = match spec.scope {
    InstallScope::User => None,
    InstallScope::Project => Some(normalize_dir_path(
      spec.project_path.as_deref().unwrap_or_default(),
      &env.home,
    )?),
  };
  let path = env.agent_profile(app, spec.scope, project_root.as_deref())?;
  // Check the directory, not the file: a symlink into the library must stay installable.
  if path
    .parent()
    .map(|parent| is_within(parent, &env.youskill_root))
    .unwrap_or(false)
  {
    return Err(format!(
      "{} points inside the YouSkill directory",
      app.display_name
    ));
  }
  Ok(ResolvedTarget {
    scope: spec.scope,
    project_path: project_root.map(|p| path_to_string(&p)),
    path,
    agent_id: spec.agent_id.clone(),
  })
}

/// Every installed agent app that reads the file of this install.
pub fn agents_reading(env: &Env, install: &InstallRecord) -> Vec<String> {
  let file = Path::new(&install.path);
  let project_root = install.project_path.as_deref().map(PathBuf::from);
  env
    .agent_apps
    .iter()
    .filter(|app| {
      env
        .agent_profile(app, install.scope, project_root.as_deref())
        .map(|path| same_path(&path, file))
        .unwrap_or(false)
    })
    .map(|app| app.id.clone())
    .collect()
}

/// Which agent instruction location (if any) a file is: user level first, then registered
/// projects, then a project inferred from the path suffix.
pub fn classify_profile(env: &Env, file: &Path) -> Option<AgentRootMatch> {
  let user_ids: Vec<String> = env
    .agent_apps
    .iter()
    .filter(|app| {
      env
        .agent_profile(app, InstallScope::User, None)
        .map(|path| same_path(&path, file))
        .unwrap_or(false)
    })
    .map(|app| app.id.clone())
    .collect();
  if !user_ids.is_empty() {
    return Some(AgentRootMatch {
      scope: InstallScope::User,
      project_path: None,
      agent_ids: user_ids,
      registered_project: false,
    });
  }

  for project in &env.projects {
    let root = Path::new(&project.path);
    let ids: Vec<String> = env
      .agent_apps
      .iter()
      .filter(|app| {
        env
          .agent_profile(app, InstallScope::Project, Some(root))
          .map(|path| same_path(&path, file))
          .unwrap_or(false)
      })
      .map(|app| app.id.clone())
      .collect();
    if !ids.is_empty() {
      return Some(AgentRootMatch {
        scope: InstallScope::Project,
        project_path: Some(project.path.clone()),
        agent_ids: ids,
        registered_project: true,
      });
    }
  }

  let file_text = file.to_string_lossy().replace('\\', "/");
  let mut inferred: Option<(String, Vec<String>)> = None;
  for app in &env.agent_apps {
    let Some(profile) = app.profile_path.as_deref() else {
      continue;
    };
    let suffix = format!("/{}", profile.trim_matches('/'));
    if let Some(root) = file_text.strip_suffix(&suffix) {
      if root.is_empty() {
        continue;
      }
      match &mut inferred {
        Some((existing, ids)) if existing == root => ids.push(app.id.clone()),
        Some(_) => {},
        None => inferred = Some((root.to_string(), vec![app.id.clone()])),
      }
    }
  }
  inferred.map(|(root, ids)| AgentRootMatch {
    scope: InstallScope::Project,
    project_path: Some(root),
    agent_ids: ids,
    registered_project: false,
  })
}

// ---------------------------------------------------------------------------
// Drift
// ---------------------------------------------------------------------------

struct HubProbe {
  state: HubState,
  hash: Option<String>,
  description: Option<String>,
}

fn probe_hub(env: &Env, name: &str, record: &InstructionRecord) -> HubProbe {
  let file = env.instruction_file(name);
  if !file.is_file() {
    return HubProbe {
      state: HubState::Missing,
      hash: None,
      description: None,
    };
  }
  let Ok(hash) = hash_file(&file) else {
    return HubProbe {
      state: HubState::Invalid,
      hash: None,
      description: None,
    };
  };
  let state = if hash != record.hash {
    HubState::Modified
  } else {
    HubState::Ok
  };
  HubProbe {
    state,
    hash: Some(hash),
    description: first_line(&file),
  }
}

/// The first line of text after any frontmatter, without heading markers.
fn first_line(file: &Path) -> Option<String> {
  let content = fs::read_to_string(file).ok()?;
  let mut lines = content.lines().map(str::trim).peekable();
  while lines.peek().map(|line| line.is_empty()).unwrap_or(false) {
    lines.next();
  }
  if lines.peek() == Some(&"---") {
    lines.next();
    for line in lines.by_ref() {
      if line == "---" {
        break;
      }
    }
  }
  let line = lines
    .find(|line| !line.is_empty())?
    .trim_start_matches('#')
    .trim();
  if line.is_empty() {
    return None;
  }
  Some(line.chars().take(DESCRIPTION_MAX_CHARS).collect())
}

/// Three-way comparison of one install target, like a skill target but for a file.
pub fn target_state(
  env: &Env,
  name: &str,
  install: &InstallRecord,
  hub_hash: Option<&str>,
) -> (TargetState, Option<String>) {
  let path = Path::new(&install.path);
  let hub_file = env.instruction_file(name);
  if is_symlink(path) {
    if symlink_points_to(path, &hub_file) {
      return if hub_file.is_file() {
        (TargetState::InSync, hub_hash.map(str::to_string))
      } else {
        (TargetState::BrokenLink, None)
      };
    }
    if !path.is_file() {
      return (TargetState::BrokenLink, None);
    }
  } else if !path.is_file() {
    return (TargetState::Missing, None);
  }
  let Ok(current) = hash_file(path) else {
    return (TargetState::Missing, None);
  };
  let state = compare_three_way(&current, hub_hash, &install.hash);
  (state, Some(current))
}

fn build_install_view(
  env: &Env,
  name: &str,
  install: &InstallRecord,
  hub_hash: Option<&str>,
) -> InstallView {
  let (state, current_hash) = target_state(env, name, install, hub_hash);
  let project_missing = install.scope == InstallScope::Project
    && !install
      .project_path
      .as_deref()
      .map(|p| Path::new(p).is_dir())
      .unwrap_or(false);
  let missing_agent_ids = install
    .agent_ids
    .iter()
    .filter(|id| env.agent(id).is_none())
    .cloned()
    .collect();
  let mut record = install.clone();
  let mut agent_ids = agents_reading(env, install);
  for id in &install.agent_ids {
    if !agent_ids.contains(id) {
      agent_ids.push(id.clone());
    }
  }
  if !agent_ids.is_empty() {
    record.agent_ids = agent_ids;
  }
  InstallView {
    record,
    state,
    current_hash,
    project_missing,
    missing_agent_ids,
    link_target: symlink_target_string(Path::new(&install.path)),
  }
}

pub fn build_view(env: &Env, name: &str, record: &InstructionRecord) -> InstructionView {
  let probe = probe_hub(env, name, record);
  let installs: Vec<InstallView> = record
    .installs
    .iter()
    .map(|install| build_install_view(env, name, install, probe.hash.as_deref()))
    .collect();
  let has_drift = probe.state != HubState::Ok
    || installs
      .iter()
      .any(|install| install.state != TargetState::InSync);
  InstructionView {
    id: name.to_string(),
    name: record.name.clone(),
    hub_path: path_to_string(&env.instruction_file(name)),
    source: record.source.clone(),
    description: probe.description,
    hash: record.hash.clone(),
    hub_hash: probe.hash,
    hub_state: probe.state,
    imported_at: record.imported_at.clone(),
    updated_at: record.updated_at.clone(),
    installs,
    has_drift,
  }
}

fn view_of(env: &Env, name: &str) -> Result<InstructionView, String> {
  Ok(build_view(env, name, &get_record(env, name)?))
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

pub fn list_instructions(env: &Env) -> Result<Vec<InstructionView>, String> {
  migrate_named_records(env)?;
  let lock = read_lock(env)?;
  Ok(
    lock
      .instructions
      .iter()
      .map(|(id, record)| build_view(env, id, record))
      .collect(),
  )
}

pub fn get_instruction(env: &Env, id: &str) -> Result<Option<InstructionView>, String> {
  Ok(
    read_lock(env)?
      .instructions
      .get(id)
      .map(|record| build_view(env, id, record)),
  )
}

pub fn read_instruction(env: &Env, id: &str) -> Result<String, String> {
  let file = env.instruction_file(id);
  fs::read_to_string(&file).map_err(|e| format!("Failed to read {}: {}", file.to_string_lossy(), e))
}

fn new_id() -> String {
  Uuid::new_v4().to_string()
}

fn is_id(key: &str) -> bool {
  Uuid::parse_str(key).is_ok()
}

/// Records from before templates had ids were keyed by a generated name, and every agent
/// file had been copied into the library. A record that only mirrored one agent file is
/// dropped again (the agent file stays, the copy goes to the trash); any other gets an id
/// and keeps its key as display name.
fn migrate_named_records(env: &Env) -> Result<(), String> {
  let lock = read_lock(env)?;
  let legacy: Vec<(String, InstructionRecord)> = lock
    .instructions
    .iter()
    .filter(|(key, _)| !is_id(key))
    .map(|(key, record)| (key.clone(), record.clone()))
    .collect();
  if legacy.is_empty() {
    return Ok(());
  }
  let _ops = ops_guard(env)?;
  let mut renamed: Vec<(String, String)> = Vec::new();
  let mut dropped: Vec<String> = Vec::new();
  for (key, record) in legacy {
    let old_file = env.instruction_file(&key);
    let mirrors_one_file = record.installs.len() <= 1
      && record
        .installs
        .iter()
        .all(|install| install.mode == InstallMode::Copy && install.hash == record.hash);
    let adopted = match &record.source {
      InstructionSource::Agent { .. } => true,
      InstructionSource::None => record.installs.len() == 1,
      _ => false,
    };
    if adopted && mirrors_one_file {
      if old_file.exists() || is_symlink(&old_file) {
        trash_file(env, &old_file)?;
      }
      dropped.push(key);
      continue;
    }
    let id = new_id();
    let new_file = env.instruction_file(&id);
    if old_file.is_file() {
      fs::rename(&old_file, &new_file).map_err(|e| e.to_string())?;
      for install in &record.installs {
        let target = Path::new(&install.path);
        if is_symlink(target) && symlink_points_to(target, &old_file) {
          let _ = write_target(&new_file, target, InstallMode::Symlink);
        }
      }
    }
    renamed.push((key, id));
  }
  update_lock(env, |lock| {
    for key in &dropped {
      lock.instructions.remove(key);
    }
    for (key, id) in &renamed {
      if let Some(mut record) = lock.instructions.remove(key) {
        record.name = key.clone();
        lock.instructions.insert(id.clone(), record);
      }
    }
    Ok(())
  })
}

/// `<name>` of a `<name>.md` library file.
fn library_name(file: &Path) -> Option<String> {
  let file_name = file.file_name()?.to_string_lossy();
  let name = file_name.strip_suffix(".md")?;
  if name.is_empty() {
    return None;
  }
  Some(name.to_string())
}

/// Move a library file into `~/.youskill/.trash/<file>-<stamp>` so a replacement can be
/// undone.
fn trash_file(env: &Env, file: &Path) -> Result<(), String> {
  if is_symlink(file) {
    return remove_path_any(file);
  }
  let name = file
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| "instruction.md".to_string());
  fs::create_dir_all(&env.trash_root).map_err(|e| e.to_string())?;
  let mut dest = env
    .trash_root
    .join(format!("{}-{}", name, now_file_stamp()));
  let mut counter = 1;
  while dest.exists() {
    dest = env
      .trash_root
      .join(format!("{}-{}-{}", name, now_file_stamp(), counter));
    counter += 1;
  }
  if fs::rename(file, &dest).is_err() {
    fs::copy(file, &dest).map_err(|e| e.to_string())?;
    fs::remove_file(file).map_err(|e| e.to_string())?;
  }
  Ok(())
}

const NAME_MAX_CHARS: usize = 80;

/// Display names are free text: trimmed, one line, not empty.
fn validate_name(name: &str) -> Result<String, String> {
  let name = name.trim();
  if name.is_empty() {
    return Err("Name must not be empty".to_string());
  }
  if name.contains(['\n', '\r']) {
    return Err("Name must be one line".to_string());
  }
  if name.chars().count() > NAME_MAX_CHARS {
    return Err(format!(
      "Name must be at most {} characters",
      NAME_MAX_CHARS
    ));
  }
  Ok(name.to_string())
}

/// Copy Markdown files into the library as new templates.
pub fn import_instructions(
  env: &Env,
  items: Vec<InstructionImportItem>,
) -> Result<Vec<InstructionImportOutcome>, String> {
  let _ops = ops_guard(env)?;
  let mut outcomes = Vec::new();
  let mut errors = Vec::new();
  for item in items {
    match import_one(
      env,
      &item.name,
      Path::new(item.path.trim()),
      item.source,
      true,
    ) {
      Ok(outcome) => outcomes.push(outcome),
      Err(err) => errors.push(format!("{}: {}", item.name, err)),
    }
  }
  if !errors.is_empty() {
    if outcomes.is_empty() {
      return Err(errors.join("\n"));
    }
    tracing::warn!("import finished with errors: {}", errors.join("; "));
  }
  Ok(outcomes)
}

/// Import one file as a new template. A file that is itself an agent location is
/// registered as an install (when `register` is set) so the template and the file stay
/// linked. Without an explicit `source`, the file itself is recorded as one.
fn import_one(
  env: &Env,
  name: &str,
  src: &Path,
  source: Option<InstructionSource>,
  register: bool,
) -> Result<InstructionImportOutcome, String> {
  let name = validate_name(name)?;
  if !src.is_file() {
    return Err(format!("Not a file: {}", src.to_string_lossy()));
  }
  let auto_install = if register && !is_within(src, &env.youskill_root) {
    classify_profile(env, src)
  } else {
    None
  };

  let id = new_id();
  let hub_file = env.instruction_file(&id);
  fs::create_dir_all(&env.instructions_root).map_err(|e| e.to_string())?;
  fs::copy(src, &hub_file).map_err(|e| e.to_string())?;
  let hash = hash_file(&hub_file)?;
  let timestamp = now_rfc3339();
  let src_text = path_to_string(src);
  let source = source.unwrap_or_else(|| {
    if auto_install.is_some() {
      InstructionSource::Agent {
        path: src_text.clone(),
      }
    } else {
      InstructionSource::File {
        path: src_text.clone(),
      }
    }
  });
  let mut record = InstructionRecord {
    name: name.clone(),
    source,
    hash: hash.clone(),
    imported_at: timestamp.clone(),
    updated_at: timestamp.clone(),
    installs: Vec::new(),
  };
  if let Some(matched) = &auto_install {
    upsert_install(
      &mut record,
      matched.scope,
      matched.project_path.clone(),
      &src_text,
      InstallMode::Copy,
      &matched.agent_ids,
      &hash,
      &timestamp,
    );
  }
  update_lock(env, |lock| {
    // Another template may already claim this file; the newer one takes it.
    if auto_install.is_some() {
      for other in lock.instructions.values_mut() {
        other
          .installs
          .retain(|install| !same_path(Path::new(&install.path), src));
      }
    }
    lock.instructions.insert(id.clone(), record);
    Ok(())
  })?;
  Ok(InstructionImportOutcome {
    id,
    name,
    hub_path: path_to_string(&hub_file),
    hash,
  })
}

/// Start a new template from text.
pub fn create_instruction(
  env: &Env,
  name: &str,
  content: &str,
) -> Result<InstructionImportOutcome, String> {
  let _ops = ops_guard(env)?;
  let name = validate_name(name)?;
  let id = new_id();
  let hub_file = env.instruction_file(&id);
  fs::create_dir_all(&env.instructions_root).map_err(|e| e.to_string())?;
  let body = if content.trim().is_empty() {
    format!("# {}\n", name)
  } else if content.ends_with('\n') {
    content.to_string()
  } else {
    format!("{}\n", content)
  };
  fs::write(&hub_file, body).map_err(|e| e.to_string())?;
  let hash = hash_file(&hub_file)?;
  let timestamp = now_rfc3339();
  update_lock(env, |lock| {
    lock.instructions.insert(
      id.clone(),
      InstructionRecord {
        name: name.clone(),
        source: InstructionSource::None,
        hash: hash.clone(),
        imported_at: timestamp.clone(),
        updated_at: timestamp.clone(),
        installs: Vec::new(),
      },
    );
    Ok(())
  })?;
  Ok(InstructionImportOutcome {
    id,
    name,
    hub_path: path_to_string(&hub_file),
    hash,
  })
}

pub fn rename_instruction(env: &Env, id: &str, name: &str) -> Result<InstructionView, String> {
  let name = validate_name(name)?;
  let record = update_lock(env, |lock| {
    let record = record_mut(lock, id)?;
    record.name = name;
    Ok(record.clone())
  })?;
  Ok(build_view(env, id, &record))
}

/// Accept whatever is in the library file as the current version.
pub fn accept_hub(env: &Env, name: &str) -> Result<InstructionActionResult, String> {
  let _ops = ops_guard(env)?;
  let hub_file = env.instruction_file(name);
  if !hub_file.is_file() {
    return Err(format!(
      "Library copy is missing: {}",
      hub_file.to_string_lossy()
    ));
  }
  let hash = hash_file(&hub_file)?;
  let timestamp = now_rfc3339();
  let record = update_lock(env, |lock| {
    let record = record_mut(lock, name)?;
    record.hash = hash.clone();
    record.updated_at = timestamp.clone();
    Ok(record.clone())
  })?;
  Ok(InstructionActionResult::applied(build_view(
    env, name, &record,
  )))
}

/// Remove an instruction from the library. With `remove_installs = false`, symlink targets
/// become real files first so agents keep reading them.
pub fn remove_instruction(env: &Env, name: &str, remove_installs: bool) -> Result<(), String> {
  let _ops = ops_guard(env)?;
  let record = get_record(env, name)?;
  let hub_file = env.instruction_file(name);
  let mut errors = Vec::new();
  for install in &record.installs {
    let target = Path::new(&install.path);
    if remove_installs {
      if target.exists() || is_symlink(target) {
        if let Err(err) = remove_path_any(target) {
          errors.push(format!("{}: {}", install.path, err));
        }
      }
    } else if let Err(err) = materialize_symlink(&hub_file, target) {
      errors.push(format!("{}: {}", install.path, err));
    }
  }
  if hub_file.exists() || is_symlink(&hub_file) {
    trash_file(env, &hub_file)?;
  }
  update_lock(env, |lock| {
    lock.instructions.remove(name);
    Ok(())
  })?;
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  Ok(())
}

/// Replace a symlink into the library with a copy of the library file.
fn materialize_symlink(hub_file: &Path, target: &Path) -> Result<(), String> {
  if !is_symlink(target) || !symlink_points_to(target, hub_file) {
    return Ok(());
  }
  if !hub_file.is_file() {
    return Err("library copy is missing".to_string());
  }
  remove_path_any(target)?;
  fs::copy(hub_file, target)
    .map(|_| ())
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Install / uninstall
// ---------------------------------------------------------------------------

struct TargetGroup {
  scope: InstallScope,
  project_path: Option<String>,
  path: PathBuf,
  agent_ids: Vec<String>,
}

fn group_targets(env: &Env, specs: &[InstallTargetSpec]) -> Result<Vec<TargetGroup>, String> {
  let mut groups: Vec<TargetGroup> = Vec::new();
  for spec in specs {
    let resolved = resolve_target(env, spec)?;
    if let Some(group) = groups
      .iter_mut()
      .find(|group| same_path(&group.path, &resolved.path))
    {
      if !group.agent_ids.contains(&resolved.agent_id) {
        group.agent_ids.push(resolved.agent_id);
      }
    } else {
      groups.push(TargetGroup {
        scope: resolved.scope,
        project_path: resolved.project_path,
        path: resolved.path,
        agent_ids: vec![resolved.agent_id],
      });
    }
  }
  Ok(groups)
}

enum Plan {
  Keep(InstallMode),
  Write(InstallMode),
}

/// Write the library file to `target`. Returns the effective mode (Windows falls back to a
/// copy when file symlinks are not permitted).
pub(crate) fn write_target(
  hub_file: &Path,
  target: &Path,
  mode: InstallMode,
) -> Result<InstallMode, String> {
  if target.exists() || is_symlink(target) {
    remove_path_any(target)?;
  }
  if let Some(parent) = target.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  match mode {
    InstallMode::Symlink => {
      #[cfg(unix)]
      {
        std::os::unix::fs::symlink(hub_file, target).map_err(|e| e.to_string())?;
        Ok(InstallMode::Symlink)
      }
      #[cfg(windows)]
      {
        match std::os::windows::fs::symlink_file(hub_file, target) {
          Ok(()) => Ok(InstallMode::Symlink),
          Err(_) => {
            fs::copy(hub_file, target).map_err(|e| e.to_string())?;
            Ok(InstallMode::Copy)
          },
        }
      }
    },
    InstallMode::Copy => {
      fs::copy(hub_file, target).map_err(|e| e.to_string())?;
      Ok(InstallMode::Copy)
    },
  }
}

/// Decide what to do with a target file that has no install record of this instruction.
fn classify_new_target(
  hub_file: &Path,
  path: &Path,
  hub_hash: &str,
  mode: InstallMode,
  force: bool,
) -> Result<Plan, String> {
  if !path.exists() && !is_symlink(path) {
    return Ok(Plan::Write(mode));
  }
  if is_symlink(path) {
    if symlink_points_to(path, hub_file) {
      return Ok(if mode == InstallMode::Symlink {
        Plan::Keep(InstallMode::Symlink)
      } else {
        Plan::Write(mode)
      });
    }
    if !path.exists() || force {
      return Ok(Plan::Write(mode));
    }
    return Err(format!(
      "{} is a symlink to another location",
      path.to_string_lossy()
    ));
  }
  if !path.is_file() {
    return Err(format!(
      "{} exists and is not a file",
      path.to_string_lossy()
    ));
  }
  let current = hash_file(path).unwrap_or_default();
  if current == hub_hash {
    return Ok(if mode == InstallMode::Copy {
      Plan::Keep(InstallMode::Copy)
    } else {
      Plan::Write(mode)
    });
  }
  if force {
    return Ok(Plan::Write(mode));
  }
  Err(format!(
    "{} already exists with different content; import it into the library or use force to overwrite",
    path.to_string_lossy()
  ))
}

#[allow(clippy::too_many_arguments)]
fn upsert_install(
  record: &mut InstructionRecord,
  scope: InstallScope,
  project_path: Option<String>,
  path: &str,
  mode: InstallMode,
  agent_ids: &[String],
  hash: &str,
  timestamp: &str,
) {
  if let Some(existing) = record
    .installs
    .iter_mut()
    .find(|install| same_path(Path::new(&install.path), Path::new(path)))
  {
    existing.mode = mode;
    existing.hash = hash.to_string();
    for id in agent_ids {
      if !existing.agent_ids.contains(id) {
        existing.agent_ids.push(id.clone());
      }
    }
    return;
  }
  record.installs.push(InstallRecord {
    scope,
    project_path,
    path: path.to_string(),
    mode,
    hash: hash.to_string(),
    installed_at: timestamp.to_string(),
    agent_ids: agent_ids.to_vec(),
  });
}

pub fn install_instruction(
  env: &Env,
  request: InstallRequest,
) -> Result<InstructionActionResult, String> {
  let _ops = ops_guard(env)?;
  let name = request.name.trim().to_string();
  let lock = read_lock(env)?;
  let record = lock
    .instructions
    .get(&name)
    .cloned()
    .ok_or_else(|| format!("Instruction '{}' is not in the library", name))?;
  let hub_file = env.instruction_file(&name);
  if !hub_file.is_file() {
    return Err(format!(
      "Library copy is missing: {}",
      hub_file.to_string_lossy()
    ));
  }
  if request.targets.is_empty() {
    return Err("At least one install target is required".to_string());
  }
  let requested_mode = request.mode.unwrap_or_default();
  let hub_hash = hash_file(&hub_file)?;
  let groups = group_targets(env, &request.targets)?;

  let mut blockers = Vec::new();
  let mut plans: Vec<(usize, Plan)> = Vec::new();
  // (other instruction, path) pairs whose install record this install takes over.
  let mut displaced: Vec<(String, String)> = Vec::new();
  for (index, group) in groups.iter().enumerate() {
    let existing = record
      .installs
      .iter()
      .find(|install| same_path(Path::new(&install.path), &group.path));
    let plan = if let Some(existing) = existing {
      let (state, _) = target_state(env, &name, existing, Some(&hub_hash));
      let mode = request.mode.unwrap_or(existing.mode);
      if mode != existing.mode {
        if !request.force {
          blockers.push(format!(
            "{} is already installed as {:?}; use force to switch to {:?}",
            existing.path, existing.mode, mode
          ));
          continue;
        }
        if matches!(state, TargetState::Modified | TargetState::Conflict) && !request.force {
          blockers.push(format!("{} has local changes", existing.path));
          continue;
        }
        Plan::Write(mode)
      } else {
        match state {
          TargetState::Missing | TargetState::BrokenLink => Plan::Write(mode),
          _ => Plan::Keep(mode),
        }
      }
    } else {
      // One file per agent: another instruction may already be deployed there.
      let occupant = lock.instructions.iter().find(|(other, other_record)| {
        *other != &name
          && other_record
            .installs
            .iter()
            .any(|install| same_path(Path::new(&install.path), &group.path))
      });
      if let Some((other, other_record)) = occupant {
        if !request.force {
          blockers.push(format!(
            "{} is occupied by '{}'; use force to replace it",
            group.path.to_string_lossy(),
            other_record.name
          ));
          continue;
        }
        displaced.push((other.clone(), path_to_string(&group.path)));
        Plan::Write(requested_mode)
      } else {
        match classify_new_target(
          &hub_file,
          &group.path,
          &hub_hash,
          requested_mode,
          request.force,
        ) {
          Ok(plan) => plan,
          Err(blocker) => {
            blockers.push(blocker);
            continue;
          },
        }
      }
    };
    plans.push((index, plan));
  }

  if !blockers.is_empty() {
    return Ok(InstructionActionResult::blocked(
      blockers,
      Some(build_view(env, &name, &record)),
    ));
  }

  let mut errors = Vec::new();
  let mut effective: BTreeMap<usize, InstallMode> = BTreeMap::new();
  for (index, plan) in plans {
    let group = &groups[index];
    match plan {
      Plan::Keep(mode) => {
        effective.insert(index, mode);
      },
      Plan::Write(mode) => match write_target(&hub_file, &group.path, mode) {
        Ok(mode) => {
          effective.insert(index, mode);
        },
        Err(err) => errors.push(format!("{}: {}", group.path.to_string_lossy(), err)),
      },
    }
  }

  let timestamp = now_rfc3339();
  update_lock(env, |lock| {
    for (other, path) in &displaced {
      if let Some(other_record) = lock.instructions.get_mut(other) {
        other_record
          .installs
          .retain(|install| !same_path(Path::new(&install.path), Path::new(path)));
      }
    }
    let record = record_mut(lock, &name)?;
    for (index, mode) in &effective {
      let group = &groups[*index];
      upsert_install(
        record,
        group.scope,
        group.project_path.clone(),
        &path_to_string(&group.path),
        *mode,
        &group.agent_ids,
        &hub_hash,
        &timestamp,
      );
    }
    Ok(())
  })?;

  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  Ok(InstructionActionResult::applied(view_of(env, &name)?))
}

pub fn uninstall_instruction(
  env: &Env,
  request: UninstallRequest,
) -> Result<InstructionActionResult, String> {
  let _ops = ops_guard(env)?;
  let name = request.name.trim().to_string();
  let record = get_record(env, &name)?;
  let hub_hash = hash_file(&env.instruction_file(&name)).ok();

  // (install path, agent ids to drop)
  let mut removals: Vec<(String, Vec<String>)> = Vec::new();
  for spec in &request.targets {
    let project_root = spec
      .project_path
      .as_deref()
      .map(|p| normalize_dir_path(p, &env.home).unwrap_or_else(|_| PathBuf::from(p.trim())));
    for install in &record.installs {
      if install.scope != spec.scope {
        continue;
      }
      if spec.scope == InstallScope::Project {
        let same_project = match (&install.project_path, &project_root) {
          (Some(a), Some(b)) => same_path(Path::new(a), b),
          _ => false,
        };
        if !same_project {
          continue;
        }
      }
      if !install.agent_ids.contains(&spec.agent_id) {
        continue;
      }
      match removals.iter_mut().find(|(path, _)| path == &install.path) {
        Some((_, ids)) => {
          if !ids.contains(&spec.agent_id) {
            ids.push(spec.agent_id.clone());
          }
        },
        None => removals.push((install.path.clone(), vec![spec.agent_id.clone()])),
      }
    }
  }
  for path in &request.paths {
    let wanted = Path::new(path.trim());
    for install in &record.installs {
      let recorded = Path::new(&install.path);
      if recorded != wanted && !same_path(recorded, wanted) {
        continue;
      }
      match removals.iter_mut().find(|(p, _)| p == &install.path) {
        Some((_, ids)) => {
          for id in &install.agent_ids {
            if !ids.contains(id) {
              ids.push(id.clone());
            }
          }
        },
        None => removals.push((install.path.clone(), install.agent_ids.clone())),
      }
    }
  }

  let mut blockers = Vec::new();
  let mut files_to_remove: Vec<PathBuf> = Vec::new();
  for (path, ids) in &removals {
    let Some(install) = record.installs.iter().find(|i| &i.path == path) else {
      continue;
    };
    if install.agent_ids.iter().any(|id| !ids.contains(id)) {
      continue;
    }
    let target = Path::new(path);
    if !target.exists() && !is_symlink(target) {
      continue;
    }
    if is_symlink(target) {
      files_to_remove.push(target.to_path_buf());
      continue;
    }
    let (state, _) = target_state(env, &name, install, hub_hash.as_deref());
    match state {
      TargetState::Modified | TargetState::Conflict if !request.force => {
        blockers.push(format!("{} has local changes", path));
      },
      _ => files_to_remove.push(target.to_path_buf()),
    }
  }
  if !blockers.is_empty() {
    return Ok(InstructionActionResult::blocked(
      blockers,
      Some(build_view(env, &name, &record)),
    ));
  }

  let mut errors = Vec::new();
  for path in &files_to_remove {
    if let Err(err) = remove_path_any(path) {
      errors.push(format!("{}: {}", path.to_string_lossy(), err));
    }
  }
  update_lock(env, |lock| {
    let record = record_mut(lock, &name)?;
    for (path, ids) in &removals {
      if let Some(install) = record.installs.iter_mut().find(|i| &i.path == path) {
        install.agent_ids.retain(|id| !ids.contains(id));
      }
    }
    record
      .installs
      .retain(|install| !install.agent_ids.is_empty());
    Ok(())
  })?;
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  Ok(InstructionActionResult::applied(view_of(env, &name)?))
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

/// Push the library file to copy-mode targets. Without `force`, edited targets are left
/// alone and reported as blockers.
pub fn push_targets(
  env: &Env,
  name: &str,
  only: Option<&[String]>,
  force: bool,
) -> Result<InstructionActionResult, String> {
  let record = get_record(env, name)?;
  let hub_file = env.instruction_file(name);
  if !hub_file.is_file() {
    return Err(format!(
      "Library copy is missing: {}",
      hub_file.to_string_lossy()
    ));
  }
  let hub_hash = hash_file(&hub_file)?;

  let mut blockers = Vec::new();
  let mut errors = Vec::new();
  let mut pushed: Vec<String> = Vec::new();
  for install in &record.installs {
    if let Some(only) = only {
      if !only
        .iter()
        .any(|p| same_path(Path::new(p), Path::new(&install.path)))
      {
        continue;
      }
    }
    let target = Path::new(&install.path);
    if is_symlink(target) {
      if symlink_points_to(target, &hub_file) {
        pushed.push(install.path.clone());
      }
      continue;
    }
    let (state, _) = target_state(env, name, install, Some(&hub_hash));
    let overwrite = match state {
      TargetState::InSync => {
        pushed.push(install.path.clone());
        false
      },
      TargetState::Outdated | TargetState::Missing => true,
      TargetState::Modified | TargetState::Conflict => {
        if !force {
          blockers.push(format!("{} has local changes", install.path));
        }
        force
      },
      TargetState::BrokenLink => {
        errors.push(format!("{} is a broken symlink", install.path));
        false
      },
    };
    if overwrite {
      match write_target(&hub_file, target, InstallMode::Copy) {
        Ok(_) => pushed.push(install.path.clone()),
        Err(err) => errors.push(format!("{}: {}", install.path, err)),
      }
    }
  }

  if !pushed.is_empty() {
    update_lock(env, |lock| {
      let record = record_mut(lock, name)?;
      for install in &mut record.installs {
        if pushed.contains(&install.path) {
          install.hash = hub_hash.clone();
        }
      }
      Ok(())
    })?;
  }
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  let view = view_of(env, name)?;
  if blockers.is_empty() {
    Ok(InstructionActionResult::applied(view))
  } else {
    Ok(InstructionActionResult::blocked(blockers, Some(view)))
  }
}

/// Take the content of one copy target as the new library version.
pub fn adopt_target(
  env: &Env,
  name: &str,
  path: &str,
  force: bool,
) -> Result<InstructionActionResult, String> {
  let record = get_record(env, name)?;
  let install = record
    .installs
    .iter()
    .find(|install| same_path(Path::new(&install.path), Path::new(path)))
    .ok_or_else(|| format!("{} is not an install target of '{}'", path, name))?;
  let target = Path::new(&install.path);
  if is_symlink(target) {
    return Err("Symlink targets already mirror the library".to_string());
  }
  if !target.is_file() {
    return Err(format!("{} does not exist", install.path));
  }
  let probe = probe_hub(env, name, &record);
  if probe.state == HubState::Modified && !force {
    return Ok(InstructionActionResult::blocked(
      vec![format!("Library copy of '{}' has local changes", name)],
      Some(build_view(env, name, &record)),
    ));
  }
  let adopted = install.path.clone();
  let new_hash = replace_hub_file(env, name, target)?;
  update_lock(env, |lock| {
    let record = record_mut(lock, name)?;
    record.hash = new_hash.clone();
    record.updated_at = now_rfc3339();
    if let Some(install) = record
      .installs
      .iter_mut()
      .find(|install| install.path == adopted)
    {
      install.hash = new_hash.clone();
    }
    Ok(())
  })?;
  Ok(InstructionActionResult::applied(view_of(env, name)?))
}

/// Replace the library file with `src` (the old copy goes to the trash); returns the hash.
fn replace_hub_file(env: &Env, name: &str, src: &Path) -> Result<String, String> {
  let hub_file = env.instruction_file(name);
  if hub_file.exists() || is_symlink(&hub_file) {
    trash_file(env, &hub_file)?;
  }
  fs::create_dir_all(&env.instructions_root).map_err(|e| e.to_string())?;
  fs::copy(src, &hub_file).map_err(|e| e.to_string())?;
  hash_file(&hub_file)
}

pub fn sync_instruction(
  env: &Env,
  name: &str,
  action: SyncAction,
) -> Result<InstructionActionResult, String> {
  match action {
    SyncAction::PushTargets { targets, force } => {
      let _ops = ops_guard(env)?;
      push_targets(env, name, targets.as_deref(), force)
    },
    SyncAction::AdoptTarget { path, force } => {
      let _ops = ops_guard(env)?;
      adopt_target(env, name, &path, force)
    },
    SyncAction::AcceptHub => accept_hub(env, name),
    SyncAction::PullSource { .. } | SyncAction::PushSource { .. } => {
      Err("Instructions have no source to sync with".to_string())
    },
  }
}

pub fn diff_instruction(env: &Env, name: &str, path: &str) -> Result<SkillDiff, String> {
  let hub_file = env.instruction_file(name);
  let target = Path::new(path);
  let rel = target
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| format!("{}.md", name));
  let file = diff_files(&rel, &hub_file, target)?;
  let unchanged = usize::from(file.is_none());
  Ok(SkillDiff {
    name: name.to_string(),
    left_label: path_to_string(&hub_file),
    right_label: path.to_string(),
    files: file.into_iter().collect(),
    unchanged,
  })
}

// ---------------------------------------------------------------------------
// Agent files
// ---------------------------------------------------------------------------

/// Every instruction file that exists at an agent location: user level and each registered
/// project whose folder exists. Keyed by path; agents that read the same file are merged.
fn agent_files(env: &Env) -> Vec<(PathBuf, AgentRootMatch)> {
  let mut found: Vec<(PathBuf, AgentRootMatch)> = Vec::new();
  let mut add = |path: PathBuf, location: AgentRootMatch| {
    if let Some((_, existing)) = found.iter_mut().find(|(known, _)| same_entry(known, &path)) {
      for id in location.agent_ids {
        if !existing.agent_ids.contains(&id) {
          existing.agent_ids.push(id);
        }
      }
    } else {
      found.push((path, location));
    }
  };
  for app in &env.agent_apps {
    if let Ok(path) = env.agent_profile(app, InstallScope::User, None) {
      if path.is_file() {
        add(
          path,
          AgentRootMatch {
            scope: InstallScope::User,
            project_path: None,
            agent_ids: vec![app.id.clone()],
            registered_project: false,
          },
        );
      }
    }
  }
  for project in &env.projects {
    let root = Path::new(&project.path);
    if !root.is_dir() {
      continue;
    }
    for app in &env.agent_apps {
      if let Ok(path) = env.agent_profile(app, InstallScope::Project, Some(root)) {
        if path.is_file() {
          add(
            path,
            AgentRootMatch {
              scope: InstallScope::Project,
              project_path: Some(project.path.clone()),
              agent_ids: vec![app.id.clone()],
              registered_project: true,
            },
          );
        }
      }
    }
  }
  found
}

/// Template id a symlink points to, if it points into the library at all.
fn linked_template(env: &Env, link: &Path) -> Option<String> {
  let resolved = symlink_target(link)?;
  if !is_within(&resolved, &env.instructions_root) {
    return None;
  }
  library_name(&resolved).filter(|id| is_id(id))
}

/// The agent files that exist, each with the template it belongs to (by install record,
/// or by pointing into the library) and how it compares to that template.
pub fn list_instruction_files(env: &Env) -> Result<Vec<AgentFileView>, String> {
  let lock = read_lock(env)?;
  let hub_hashes: BTreeMap<String, String> = lock
    .instructions
    .keys()
    .filter_map(|id| {
      hash_file(&env.instruction_file(id))
        .ok()
        .map(|hash| (id.clone(), hash))
    })
    .collect();
  let mut files: Vec<AgentFileView> = agent_files(env)
    .into_iter()
    .map(|(path, location)| {
      let recorded = lock.instructions.iter().find_map(|(id, record)| {
        record
          .installs
          .iter()
          .find(|install| same_path(Path::new(&install.path), &path))
          .map(|install| (id.clone(), record, install))
      });
      let template = match recorded {
        Some((id, record, install)) => {
          let (state, _) = target_state(env, &id, install, hub_hashes.get(&id).map(String::as_str));
          Some(AgentFileTemplate {
            id,
            name: record.name.clone(),
            state,
          })
        },
        None => linked_template(env, &path).and_then(|id| {
          let state = if env.instruction_file(&id).is_file() {
            TargetState::InSync
          } else {
            TargetState::BrokenLink
          };
          lock.instructions.get(&id).map(|record| AgentFileTemplate {
            id,
            name: record.name.clone(),
            state,
          })
        }),
      };
      AgentFileView {
        file_name: path
          .file_name()
          .map(|n| n.to_string_lossy().to_string())
          .unwrap_or_default(),
        path: path_to_string(&path),
        scope: location.scope,
        project_path: location.project_path,
        agent_ids: location.agent_ids,
        hash: hash_file(&path).ok(),
        template,
        link_target: symlink_target_string(&path),
        references: references_of(&path, &env.home),
      }
    })
    .collect();
  files.sort_by(|a, b| {
    let rank = |file: &AgentFileView| match file.scope {
      InstallScope::User => 0,
      InstallScope::Project => 1,
    };
    rank(a).cmp(&rank(b)).then_with(|| a.path.cmp(&b.path))
  });
  Ok(files)
}

/// The agent file at `path`, refusing anything that is not an agent location.
fn agent_file(env: &Env, path: &str) -> Result<PathBuf, String> {
  let wanted = expand_home_with(path.trim(), &env.home);
  agent_files(env)
    .into_iter()
    .map(|(known, _)| known)
    .find(|known| same_path(known, &wanted))
    .ok_or_else(|| format!("{} is not an agent instruction file", path))
}

pub fn read_instruction_file(env: &Env, path: &str) -> Result<String, String> {
  let file = agent_file(env, path)?;
  fs::read_to_string(&file).map_err(|e| format!("Failed to read {}: {}", file.to_string_lossy(), e))
}

/// Write an agent file in place. A symlink into the library writes the template instead,
/// so the change is accepted there and reaches every other copy that was in sync.
pub fn write_instruction_file(env: &Env, path: &str, content: &str) -> Result<(), String> {
  let file = agent_file(env, path)?;
  if let Some(id) = linked_template(env, &file) {
    return write_instruction(env, &id, content).map(|_| ());
  }
  let body = if content.is_empty() || content.ends_with('\n') {
    content.to_string()
  } else {
    format!("{}\n", content)
  };
  fs::write(&file, body).map_err(|e| format!("Failed to write {}: {}", file.to_string_lossy(), e))
}

fn file_stem(path: &Path) -> String {
  path
    .file_stem()
    .map(|s| s.to_string_lossy().to_string())
    .unwrap_or_default()
}

/// `AGENTS`, `CLAUDE`, `GEMINI`: conventional file names that say nothing on their own.
fn is_upper(stem: &str) -> bool {
  !stem.is_empty()
    && stem
      .chars()
      .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A valid library name derived from free text: lower case, runs of other characters
/// become one `-`.
fn slug(text: &str) -> String {
  let mut out = String::new();
  let mut dash = false;
  for c in text.chars() {
    if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
      out.push(c.to_ascii_lowercase());
      dash = false;
    } else if !dash && !out.is_empty() {
      out.push('-');
      dash = true;
    }
  }
  let out = out.trim_matches(|c| c == '-' || c == '.').to_string();
  if out.is_empty() {
    "instruction".to_string()
  } else {
    out
  }
}

fn unique_name(base: &str, used: &[String]) -> String {
  let mut candidate = base.to_string();
  let mut counter = 2;
  while used.contains(&candidate.to_lowercase()) {
    candidate = format!("{}-{}", base, counter);
    counter += 1;
  }
  candidate
}

// ---------------------------------------------------------------------------
// Detection: folders, files and GitHub repositories
// ---------------------------------------------------------------------------

const DETECT_MAX_DEPTH: usize = 6;
const DETECT_MAX_FILES: usize = 200;
const MARKDOWN_EXTENSIONS: [&str; 3] = ["md", "markdown", "mdc"];
/// Directories that never hold instruction files worth listing.
const SKIPPED_DIRS: [&str; 7] = [
  ".git",
  "node_modules",
  "target",
  "dist",
  "build",
  "vendor",
  ".youskill",
];

fn is_markdown(path: &Path) -> bool {
  path
    .extension()
    .map(|ext| {
      MARKDOWN_EXTENSIONS
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known))
    })
    .unwrap_or(false)
}

/// Hidden directories worth descending into: `.github` and whatever an agent's project
/// instruction file lives in. Other dotfolders (`.venv`, `.next`, ...) are skipped.
fn allowed_hidden_dirs(env: &Env) -> Vec<String> {
  let mut dirs = vec![".github".to_string()];
  for app in &env.agent_apps {
    let Some(profile) = app.profile_path.as_deref() else {
      continue;
    };
    let parents: Vec<String> = Path::new(profile)
      .parent()
      .map(|parent| {
        parent
          .components()
          .map(|c| c.as_os_str().to_string_lossy().to_string())
          .collect()
      })
      .unwrap_or_default();
    for dir in parents {
      if dir.starts_with('.') && dir.len() > 1 && !dirs.contains(&dir) {
        dirs.push(dir);
      }
    }
  }
  dirs
}

/// Markdown files under `root` (or `root` itself when it is a file), shallowest first.
fn find_markdown_files(env: &Env, root: &Path) -> Result<Vec<PathBuf>, String> {
  if root.is_file() {
    return Ok(vec![root.to_path_buf()]);
  }
  if !root.is_dir() {
    return Err(format!("Path does not exist: {}", root.to_string_lossy()));
  }
  let hidden = allowed_hidden_dirs(env);
  let mut out: Vec<PathBuf> = Vec::new();
  let mut walker = WalkDir::new(root)
    .follow_links(false)
    .max_depth(DETECT_MAX_DEPTH)
    .sort_by_file_name()
    .into_iter();
  while let Some(entry) = walker.next() {
    let Ok(entry) = entry else {
      continue;
    };
    let name = entry.file_name().to_string_lossy().to_string();
    if entry.file_type().is_dir() {
      let skip = entry.depth() > 0
        && (SKIPPED_DIRS.contains(&name.as_str())
          || (name.starts_with('.') && !hidden.contains(&name)));
      if skip {
        walker.skip_current_dir();
      }
      continue;
    }
    if !entry.file_type().is_file() || !is_markdown(entry.path()) {
      continue;
    }
    out.push(entry.path().to_path_buf());
    if out.len() >= DETECT_MAX_FILES {
      break;
    }
  }
  out.sort_by_key(|path| (path.components().count(), path.clone()));
  Ok(out)
}

/// `AGENTS.md` at the top of `repo` → `repo-agents`, `packages/x/CLAUDE.md` → `x-claude`,
/// `prompts/python.md` → `python`.
fn detected_name(file: &Path, base: &Path, base_label: &str) -> String {
  let stem = file_stem(file);
  if !is_upper(&stem) {
    return slug(&stem);
  }
  let parent = file.parent().unwrap_or(base);
  let label = if same_path(parent, base) {
    base_label.to_string()
  } else {
    parent
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| base_label.to_string())
  };
  slug(&format!("{}-{}", label, stem))
}

/// List the Markdown files under `root` as import candidates; `base` is where relative
/// paths start and `base_label` names it for suggested names.
fn detect_in(
  env: &Env,
  root: &Path,
  base: &Path,
  base_label: &str,
  used: &mut Vec<String>,
  source: impl Fn(&str) -> Option<InstructionSource>,
) -> Result<Vec<DetectedInstruction>, String> {
  let mut items = Vec::new();
  for file in find_markdown_files(env, root)? {
    let rel_path = file
      .strip_prefix(base)
      .map(|p| p.to_string_lossy().replace('\\', "/"))
      .unwrap_or_else(|_| path_to_string(&file));
    let name = unique_name(&detected_name(&file, base, base_label), used);
    used.push(name.to_lowercase());
    items.push(DetectedInstruction {
      name,
      path: path_to_string(&file),
      source: source(&rel_path),
      rel_path,
      file_name: file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default(),
    });
  }
  Ok(items)
}

fn folder_label(path: &Path) -> String {
  path
    .file_name()
    .map(|n| n.to_string_lossy().trim_start_matches('.').to_string())
    .filter(|label| !label.is_empty())
    .unwrap_or_else(|| "instruction".to_string())
}

/// Markdown files in the given files and folders. Suggested names are unique across the
/// result but may match existing entries, which import can overwrite on request.
pub fn detect_instruction_files(
  env: &Env,
  paths: &[String],
) -> Result<Vec<DetectedInstruction>, String> {
  let mut used: Vec<String> = Vec::new();
  let mut items = Vec::new();
  for raw in paths {
    let path = expand_home_with(raw.trim(), &env.home);
    let base = if path.is_file() {
      path.parent().map(Path::to_path_buf).unwrap_or(path.clone())
    } else {
      path.clone()
    };
    let label = folder_label(&base);
    items.extend(detect_in(env, &path, &base, &label, &mut used, |_| None)?);
  }
  Ok(items)
}

/// Download a repository (optionally a `/tree/` or `/blob/` path inside it) and list its
/// Markdown files. The archive stays in a temp directory until the import copies from it.
pub async fn detect_instruction_github(
  env: &Env,
  github_path: &str,
) -> Result<Vec<DetectedInstruction>, String> {
  let reference = GithubHelper::parse_github_ref(github_path)?;
  let clone_dir = create_temp_dir(&format!(
    "detect-instructions-{}-{}",
    reference.owner, reference.repo
  ))?;
  let branch = GithubHelper::clone_repo_to(
    &reference.owner,
    &reference.repo,
    reference.branch.as_deref(),
    &clone_dir,
  )
  .await?;
  let repo = format!("{}/{}", reference.owner, reference.repo);
  let root = match &reference.subpath {
    Some(subpath) => {
      let inside = clone_dir.join(subpath);
      if !inside.exists() {
        return Err(format!("{} was not found in the repository", subpath));
      }
      inside
    },
    None => clone_dir.clone(),
  };
  let mut used: Vec<String> = Vec::new();
  detect_in(env, &root, &clone_dir, &reference.repo, &mut used, |rel| {
    Some(InstructionSource::Github {
      repo: repo.clone(),
      file_path: rel.to_string(),
      branch: Some(branch.clone()),
    })
  })
}

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

/// Save edited content to the library file and push it to the copy targets that were in
/// sync. Targets with local changes are reported as blockers and left alone.
pub fn write_instruction(
  env: &Env,
  name: &str,
  content: &str,
) -> Result<InstructionActionResult, String> {
  let _ops = ops_guard(env)?;
  get_record(env, name)?;
  let hub_file = env.instruction_file(name);
  fs::create_dir_all(&env.instructions_root).map_err(|e| e.to_string())?;
  let body = if content.is_empty() || content.ends_with('\n') {
    content.to_string()
  } else {
    format!("{}\n", content)
  };
  fs::write(&hub_file, body).map_err(|e| e.to_string())?;
  let hash = hash_file(&hub_file)?;
  let timestamp = now_rfc3339();
  update_lock(env, |lock| {
    let record = record_mut(lock, name)?;
    record.hash = hash.clone();
    record.updated_at = timestamp.clone();
    Ok(())
  })?;
  push_targets(env, name, None, false)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::{AgentApp, UserProject};

  fn app(id: &str, profile: &str, global_profile: &str) -> AgentApp {
    AgentApp {
      id: id.to_string(),
      display_name: id.to_string(),
      project_path: Some(format!(".{}/skills", id)),
      global_path: Some(format!("~/.{}/skills", id)),
      detect_path: None,
      profile_path: Some(profile.to_string()),
      global_profile_path: Some(global_profile.to_string()),
      is_user_custom: false,
    }
  }

  fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
  }

  fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
  }

  /// Claude Code with its own file, Codex and the shared `agents` app both on `AGENTS.md`,
  /// and one registered project.
  fn env(tmp: &Path) -> Env {
    let project = tmp.join("proj");
    fs::create_dir_all(&project).unwrap();
    Env::for_test(
      tmp,
      vec![
        app("agents", "AGENTS.md", "~/.agents/AGENTS.md"),
        app("claude-code", "CLAUDE.md", "~/.claude/CLAUDE.md"),
        app("codex", "AGENTS.md", "~/.codex/AGENTS.md"),
      ],
      vec![UserProject {
        name: "proj".to_string(),
        path: project.to_string_lossy().to_string(),
        workspace_path: None,
      }],
    )
  }

  /// Import `body` from a scratch file under `name`; returns the template id.
  fn import(env: &Env, tmp: &Path, name: &str, body: &str) -> String {
    let src = tmp.join("src").join(format!("{}.md", name));
    write(&src, body);
    let mut outcomes = import_instructions(
      env,
      vec![InstructionImportItem {
        name: name.to_string(),
        path: src.to_string_lossy().to_string(),
        source: None,
      }],
    )
    .unwrap();
    outcomes.remove(0).id
  }

  fn spec(scope: InstallScope, project: Option<&Path>, agent: &str) -> InstallTargetSpec {
    InstallTargetSpec {
      scope,
      project_path: project.map(|p| p.to_string_lossy().to_string()),
      agent_id: agent.to_string(),
    }
  }

  fn install(
    env: &Env,
    id: &str,
    targets: Vec<InstallTargetSpec>,
    mode: InstallMode,
    force: bool,
  ) -> InstructionActionResult {
    install_instruction(
      env,
      InstallRequest {
        name: id.to_string(),
        targets,
        mode: Some(mode),
        force,
      },
    )
    .unwrap()
  }

  #[test]
  fn import_install_drift_adopt_and_push() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = import(&env, tmp.path(), "rules", "# Team rules\n\nv1\n");
    assert!(is_id(&id));
    let view = get_instruction(&env, &id).unwrap().unwrap();
    assert_eq!(view.name, "rules");
    assert_eq!(view.description.as_deref(), Some("Team rules"));
    assert_eq!(view.hub_state, HubState::Ok);
    assert!(matches!(view.source, InstructionSource::File { .. }));
    assert!(view.installs.is_empty());
    // Names are free text and may repeat; ids tell templates apart.
    let again = import(&env, tmp.path(), "rules", "other\n");
    assert_ne!(again, id);
    assert_eq!(list_instructions(&env).unwrap().len(), 2);

    let result = install(
      &env,
      &id,
      vec![spec(InstallScope::User, None, "claude-code")],
      InstallMode::Copy,
      false,
    );
    assert!(result.applied);
    let target = env.home.join(".claude/CLAUDE.md");
    assert!(read(&target).ends_with("v1\n"));
    let view = result.instruction.unwrap();
    assert_eq!(view.installs.len(), 1);
    assert_eq!(view.installs[0].state, TargetState::InSync);
    assert_eq!(view.installs[0].record.agent_ids, vec!["claude-code"]);

    // Edit the target: modified, then adopted into the library.
    write(&target, "# Team rules\n\nv2 edited\n");
    let view = get_instruction(&env, &id).unwrap().unwrap();
    assert_eq!(view.installs[0].state, TargetState::Modified);
    assert!(view.has_drift);
    let adopted = sync_instruction(
      &env,
      &id,
      SyncAction::AdoptTarget {
        path: path_to_string(&target),
        force: false,
      },
    )
    .unwrap();
    assert!(adopted.applied);
    assert!(read(&env.instruction_file(&id)).ends_with("v2 edited\n"));
    assert_eq!(
      adopted.instruction.unwrap().installs[0].state,
      TargetState::InSync
    );

    // Edit the library copy: accept it, the target is outdated, push it.
    write(&env.instruction_file(&id), "# Team rules\n\nv3\n");
    let view = get_instruction(&env, &id).unwrap().unwrap();
    assert_eq!(view.hub_state, HubState::Modified);
    let accepted = sync_instruction(&env, &id, SyncAction::AcceptHub).unwrap();
    let view = accepted.instruction.unwrap();
    assert_eq!(view.hub_state, HubState::Ok);
    assert_eq!(view.installs[0].state, TargetState::Outdated);
    let pushed = sync_instruction(
      &env,
      &id,
      SyncAction::PushTargets {
        targets: None,
        force: false,
      },
    )
    .unwrap();
    assert!(pushed.applied);
    assert!(read(&target).ends_with("v3\n"));
  }

  #[test]
  fn shared_project_file_groups_agents_and_uninstalls_by_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = import(&env, tmp.path(), "rules", "rules\n");
    let project = tmp.path().join("proj");
    let result = install(
      &env,
      &id,
      vec![
        spec(InstallScope::Project, Some(&project), "codex"),
        spec(InstallScope::Project, Some(&project), "agents"),
      ],
      InstallMode::Copy,
      false,
    );
    assert!(result.applied);
    let record = get_record(&env, &id).unwrap();
    assert_eq!(record.installs.len(), 1);
    assert!(same_path(
      Path::new(&record.installs[0].path),
      &project.join("AGENTS.md")
    ));
    assert_eq!(record.installs[0].agent_ids, vec!["codex", "agents"]);
    // Claude Code reads CLAUDE.md, so it is not shown on the AGENTS.md install.
    let view = get_instruction(&env, &id).unwrap().unwrap();
    assert_eq!(view.installs[0].record.agent_ids, vec!["agents", "codex"]);

    let result = uninstall_instruction(
      &env,
      UninstallRequest {
        name: id.clone(),
        targets: vec![spec(InstallScope::Project, Some(&project), "codex")],
        paths: vec![],
        force: false,
      },
    )
    .unwrap();
    assert!(result.applied);
    assert!(project.join("AGENTS.md").is_file(), "agents still reads it");
    let result = uninstall_instruction(
      &env,
      UninstallRequest {
        name: id.clone(),
        targets: vec![],
        paths: vec![path_to_string(&project.join("AGENTS.md"))],
        force: false,
      },
    )
    .unwrap();
    assert!(result.applied);
    assert!(!project.join("AGENTS.md").exists());
    assert!(get_record(&env, &id).unwrap().installs.is_empty());
  }

  #[test]
  fn occupied_file_is_blocked_then_taken_over_with_force() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let a = import(&env, tmp.path(), "a", "A\n");
    let b = import(&env, tmp.path(), "b", "B\n");
    let project = tmp.path().join("proj");
    assert!(
      install(
        &env,
        &a,
        vec![spec(InstallScope::Project, Some(&project), "codex")],
        InstallMode::Copy,
        false
      )
      .applied
    );

    let blocked = install(
      &env,
      &b,
      vec![spec(InstallScope::Project, Some(&project), "codex")],
      InstallMode::Copy,
      false,
    );
    assert!(!blocked.applied);
    assert!(blocked.blockers[0].contains("occupied by 'a'"));
    assert_eq!(read(&project.join("AGENTS.md")), "A\n");

    let forced = install(
      &env,
      &b,
      vec![spec(InstallScope::Project, Some(&project), "codex")],
      InstallMode::Copy,
      true,
    );
    assert!(forced.applied);
    assert_eq!(read(&project.join("AGENTS.md")), "B\n");
    assert!(get_record(&env, &a).unwrap().installs.is_empty());
    assert_eq!(get_record(&env, &b).unwrap().installs.len(), 1);
  }

  #[test]
  fn hand_written_file_blocks_install_until_forced() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = import(&env, tmp.path(), "rules", "rules\n");
    let target = env.home.join(".codex/AGENTS.md");
    write(&target, "mine\n");
    let blocked = install(
      &env,
      &id,
      vec![spec(InstallScope::User, None, "codex")],
      InstallMode::Copy,
      false,
    );
    assert!(!blocked.applied);
    assert!(blocked.blockers[0].contains("different content"));
    assert_eq!(read(&target), "mine\n");
    let forced = install(
      &env,
      &id,
      vec![spec(InstallScope::User, None, "codex")],
      InstallMode::Copy,
      true,
    );
    assert!(forced.applied);
    assert_eq!(read(&target), "rules\n");
  }

  #[test]
  fn promoting_an_agent_file_registers_it_and_takes_it_over() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let source = env.home.join(".codex/AGENTS.md");
    write(&source, "# Codex\n");
    let outcome = import_instructions(
      &env,
      vec![InstructionImportItem {
        name: "Codex rules".to_string(),
        path: path_to_string(&source),
        source: None,
      }],
    )
    .unwrap()
    .remove(0);
    let view = get_instruction(&env, &outcome.id).unwrap().unwrap();
    assert_eq!(view.name, "Codex rules");
    assert_eq!(
      view.source,
      InstructionSource::Agent {
        path: path_to_string(&source)
      }
    );
    assert_eq!(view.installs.len(), 1);
    assert_eq!(view.installs[0].record.scope, InstallScope::User);
    assert_eq!(view.installs[0].record.agent_ids, vec!["codex"]);
    assert_eq!(view.installs[0].state, TargetState::InSync);
    assert!(source.is_file(), "the agent file stays in place");

    // Promoting the same file again moves the install to the new template.
    let second = import_instructions(
      &env,
      vec![InstructionImportItem {
        name: "Codex rules".to_string(),
        path: path_to_string(&source),
        source: None,
      }],
    )
    .unwrap()
    .remove(0);
    assert!(get_record(&env, &outcome.id).unwrap().installs.is_empty());
    assert_eq!(get_record(&env, &second.id).unwrap().installs.len(), 1);
  }

  #[test]
  fn reference_agent_file_lists_what_it_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let project = tmp.path().join("proj");
    write(&project.join("AGENTS.md"), "# Proj\n");
    write(&project.join("CLAUDE.md"), "@AGENTS.md\n");

    let files = list_instruction_files(&env).unwrap();
    let agents_md = files.iter().find(|f| f.file_name == "AGENTS.md").unwrap();
    let claude_md = files.iter().find(|f| f.file_name == "CLAUDE.md").unwrap();
    assert!(agents_md.references.is_empty());
    assert_eq!(
      claude_md.references,
      vec![project.join("AGENTS.md").to_string_lossy().to_string()]
    );

    let memory = crate::services::workspace_service::list_memory_files(
      &env,
      InstallScope::Project,
      project.to_str(),
    )
    .unwrap();
    let claude_md = memory.iter().find(|f| f.name == "CLAUDE.md").unwrap();
    assert_eq!(claude_md.references.len(), 1);
  }

  #[cfg(unix)]
  #[test]
  fn symlinked_agent_file_is_listed_apart_from_its_target() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let project = tmp.path().join("proj");
    write(&project.join("AGENTS.md"), "# Proj\n");
    std::os::unix::fs::symlink("AGENTS.md", project.join("CLAUDE.md")).unwrap();

    let files = list_instruction_files(&env).unwrap();
    let agents_md = files.iter().find(|f| f.file_name == "AGENTS.md").unwrap();
    let claude_md = files.iter().find(|f| f.file_name == "CLAUDE.md").unwrap();
    assert_eq!(agents_md.agent_ids, vec!["agents", "codex"]);
    assert!(agents_md.link_target.is_none());
    assert_eq!(claude_md.agent_ids, vec!["claude-code"]);
    assert_eq!(
      claude_md.link_target.as_deref(),
      Some(project.join("AGENTS.md").to_str().unwrap())
    );

    let memory = crate::services::workspace_service::list_memory_files(
      &env,
      InstallScope::Project,
      project.to_str(),
    )
    .unwrap();
    let claude_md = memory.iter().find(|f| f.name == "CLAUDE.md").unwrap();
    assert_eq!(claude_md.agent_ids, vec!["claude-code"]);
    assert!(claude_md.link_target.is_some());
  }

  #[test]
  fn agent_files_are_listed_with_their_template() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let project = tmp.path().join("proj");
    write(&env.home.join(".claude/CLAUDE.md"), "# Claude\n");
    write(&project.join("AGENTS.md"), "# Proj\n");

    let files = list_instruction_files(&env).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].scope, InstallScope::User);
    assert_eq!(files[0].agent_ids, vec!["claude-code"]);
    assert!(files[0].template.is_none());
    assert_eq!(files[1].file_name, "AGENTS.md");
    assert_eq!(files[1].agent_ids, vec!["agents", "codex"]);
    assert_eq!(
      files[1].project_path.as_deref(),
      Some(project.to_str().unwrap())
    );
    // Nothing was copied into the library.
    assert!(list_instructions(&env).unwrap().is_empty());

    let id = import(&env, tmp.path(), "rules", "# Proj\n");
    assert!(
      install(
        &env,
        &id,
        vec![spec(InstallScope::Project, Some(&project), "codex")],
        InstallMode::Copy,
        false
      )
      .applied
    );
    let files = list_instruction_files(&env).unwrap();
    let linked = files[1].template.as_ref().unwrap();
    assert_eq!(linked.id, id);
    assert_eq!(linked.name, "rules");
    assert_eq!(linked.state, TargetState::InSync);

    // Editing the file in place drifts it from the template.
    write_instruction_file(&env, &files[1].path, "# Proj edited").unwrap();
    assert_eq!(read(&project.join("AGENTS.md")), "# Proj edited\n");
    let files = list_instruction_files(&env).unwrap();
    assert_eq!(
      files[1].template.as_ref().unwrap().state,
      TargetState::Modified
    );
    assert_eq!(
      read_instruction_file(&env, &files[1].path).unwrap(),
      "# Proj edited\n"
    );
    assert!(read_instruction_file(&env, &path_to_string(&env.instruction_file(&id))).is_err());
    assert!(write_instruction_file(&env, "/etc/hosts", "x").is_err());
  }

  #[cfg(unix)]
  #[test]
  fn writing_a_linked_file_edits_the_template() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = import(&env, tmp.path(), "rules", "v1\n");
    let claude = env.home.join(".claude/CLAUDE.md");
    let codex = env.home.join(".codex/AGENTS.md");
    assert!(
      install(
        &env,
        &id,
        vec![spec(InstallScope::User, None, "claude-code")],
        InstallMode::Symlink,
        false
      )
      .applied
    );
    assert!(
      install(
        &env,
        &id,
        vec![spec(InstallScope::User, None, "codex")],
        InstallMode::Copy,
        false
      )
      .applied
    );
    write_instruction_file(&env, &path_to_string(&claude), "v2").unwrap();
    assert_eq!(read(&env.instruction_file(&id)), "v2\n");
    assert_eq!(read(&codex), "v2\n", "the copy that was in sync follows");
    let view = get_instruction(&env, &id).unwrap().unwrap();
    assert_eq!(view.hub_state, HubState::Ok);
    assert!(!view.has_drift);
    let files = list_instruction_files(&env).unwrap();
    assert!(files
      .iter()
      .all(|file| file.template.as_ref().map(|t| t.state) == Some(TargetState::InSync)));
  }

  #[test]
  fn detect_lists_markdown_files_with_suggested_names() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let repo = tmp.path().join("repo");
    write(&repo.join("AGENTS.md"), "a\n");
    write(&repo.join("README.md"), "r\n");
    write(&repo.join("packages/x/CLAUDE.md"), "c\n");
    write(&repo.join("prompts/python.md"), "p\n");
    write(&repo.join(".github/copilot-instructions.md"), "g\n");
    write(&repo.join("node_modules/dep/AGENTS.md"), "n\n");
    write(&repo.join(".venv/lib/notes.md"), "v\n");
    write(&repo.join("src/main.rs"), "fn main() {}\n");

    let items = detect_instruction_files(&env, &[path_to_string(&repo)]).unwrap();
    let rel: Vec<&str> = items.iter().map(|item| item.rel_path.as_str()).collect();
    assert_eq!(
      rel,
      vec![
        "AGENTS.md",
        "README.md",
        ".github/copilot-instructions.md",
        "prompts/python.md",
        "packages/x/CLAUDE.md",
      ]
    );
    let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
    assert_eq!(
      names,
      vec![
        "repo-agents",
        "repo-readme",
        "copilot-instructions",
        "python",
        "x-claude"
      ]
    );

    let single =
      detect_instruction_files(&env, &[path_to_string(&repo.join("prompts/python.md"))]).unwrap();
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].name, "python");
    assert_eq!(single[0].rel_path, "python.md");
    assert!(detect_instruction_files(&env, &[path_to_string(&repo.join("missing"))]).is_err());
  }

  #[test]
  fn writing_content_updates_the_hash_and_pushes_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = import(&env, tmp.path(), "rules", "v1\n");
    let target = env.home.join(".claude/CLAUDE.md");
    assert!(
      install(
        &env,
        &id,
        vec![spec(InstallScope::User, None, "claude-code")],
        InstallMode::Copy,
        false
      )
      .applied
    );

    let result = write_instruction(&env, &id, "v2").unwrap();
    assert!(result.applied);
    let view = result.instruction.unwrap();
    assert_eq!(view.hub_state, HubState::Ok);
    assert_eq!(view.installs[0].state, TargetState::InSync);
    assert_eq!(read(&target), "v2\n");
    assert_eq!(read_instruction(&env, &id).unwrap(), "v2\n");

    // A target edited by hand is reported, not overwritten.
    write(&target, "mine\n");
    let result = write_instruction(&env, &id, "v3\n").unwrap();
    assert!(!result.applied);
    assert!(result.blockers[0].contains("local changes"));
    assert_eq!(read(&target), "mine\n");
    assert_eq!(read_instruction(&env, &id).unwrap(), "v3\n");
  }

  #[cfg(unix)]
  #[test]
  fn create_rename_symlink_and_remove_keeping_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let outcome = create_instruction(&env, "fresh", "").unwrap();
    let id = outcome.id.clone();
    assert_eq!(read(Path::new(&outcome.hub_path)), "# fresh\n");
    assert!(create_instruction(&env, "  ", "again").is_err());
    let view = rename_instruction(&env, &id, "  Fresh rules ").unwrap();
    assert_eq!(view.name, "Fresh rules");
    assert_eq!(view.source, InstructionSource::None);

    let result = install(
      &env,
      &id,
      vec![spec(InstallScope::User, None, "claude-code")],
      InstallMode::Symlink,
      false,
    );
    assert!(result.applied);
    let target = env.home.join(".claude/CLAUDE.md");
    assert!(is_symlink(&target));
    assert_eq!(
      get_instruction(&env, &id).unwrap().unwrap().installs[0].state,
      TargetState::InSync
    );

    let diff = diff_instruction(&env, &id, &path_to_string(&target)).unwrap();
    assert!(diff.files.is_empty());
    assert_eq!(diff.unchanged, 1);

    remove_instruction(&env, &id, false).unwrap();
    assert!(!is_symlink(&target));
    assert_eq!(read(&target), "# fresh\n");
    assert!(get_instruction(&env, &id).unwrap().is_none());
    assert!(!env.instruction_file(&id).exists());
  }

  #[cfg(unix)]
  #[test]
  fn named_records_are_migrated_to_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let project = tmp.path().join("proj");
    let claude = env.home.join(".claude/CLAUDE.md");
    let agents = project.join("AGENTS.md");
    write(&claude, "# Claude\n");
    write(&agents, "# Proj\n");
    // A shared template linked from one file, a mirror of one project file, and one
    // created in the app.
    write(&env.instruction_file("shared"), "# Claude\n");
    write(&env.instruction_file("proj-agents"), "# Proj\n");
    write(&env.instruction_file("notes"), "# Notes\n");
    let codex = env.home.join(".codex/AGENTS.md");
    fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(env.instruction_file("shared"), &codex).unwrap();
    let record =
      |source: InstructionSource, installs: Vec<(&Path, InstallMode)>| InstructionRecord {
        name: String::new(),
        source,
        hash: hash_file(&claude).unwrap(),
        imported_at: "t".to_string(),
        updated_at: "t".to_string(),
        installs: installs
          .into_iter()
          .map(|(path, mode)| InstallRecord {
            scope: InstallScope::User,
            project_path: None,
            path: path_to_string(path),
            mode,
            hash: hash_file(&claude).unwrap(),
            installed_at: "t".to_string(),
            agent_ids: vec!["x".to_string()],
          })
          .collect(),
      };
    update_lock(&env, |lock| {
      lock.instructions.insert(
        "shared".to_string(),
        record(
          InstructionSource::Agent {
            path: path_to_string(&claude),
          },
          vec![(&claude, InstallMode::Copy), (&codex, InstallMode::Symlink)],
        ),
      );
      let mut mirror = record(
        InstructionSource::Agent {
          path: path_to_string(&agents),
        },
        vec![(&agents, InstallMode::Copy)],
      );
      mirror.hash = hash_file(&agents).unwrap();
      mirror.installs[0].hash = mirror.hash.clone();
      lock.instructions.insert("proj-agents".to_string(), mirror);
      lock
        .instructions
        .insert("notes".to_string(), record(InstructionSource::None, vec![]));
      Ok(())
    })
    .unwrap();

    let list = list_instructions(&env).unwrap();
    let mut names: Vec<&str> = list.iter().map(|item| item.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["notes", "shared"]);
    assert!(list.iter().all(|item| is_id(&item.id)));
    assert!(!env.instruction_file("shared").exists());
    assert!(!env.instruction_file("proj-agents").exists());
    assert!(agents.is_file(), "the mirrored agent file stays");
    let shared = list.iter().find(|item| item.name == "shared").unwrap();
    assert_eq!(shared.installs.len(), 2);
    assert!(symlink_points_to(&codex, &env.instruction_file(&shared.id)));
    assert!(shared
      .installs
      .iter()
      .all(|install| install.state == TargetState::InSync));
    let files = list_instruction_files(&env).unwrap();
    let proj_file = files
      .iter()
      .find(|file| file.path == path_to_string(&agents))
      .unwrap();
    assert!(proj_file.template.is_none());
  }
}
