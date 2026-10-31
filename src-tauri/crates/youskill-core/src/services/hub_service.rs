//! The central skill repository (`~/.youskill/skills`) and its lock file.

use crate::models::{
  is_skill_id, new_skill_id, ActionResult, HubSkillView, HubState, ImportItem, ImportOutcome,
  ImportPlan, InstallMode, InstallRecord, InstallScope, LockFile, SkillMatch, SkillRecord,
  SkillSource, SyncAction,
};
use crate::services::drift_service::{build_view, probe_hub};
use crate::services::env::Env;
use crate::services::install_service::{
  adopt_target, classify_agent_root, materialize_symlink, push_targets,
};
use crate::services::lock_service::{ops_guard, ops_guard_unless_held, store};
use crate::services::migration_service::migrate_skill_ids;
use crate::utils::folder::{
  copy_dir, is_staged_temp_path, move_dir, replace_dir_atomic, CopyOpts, SKILL_MD,
};
use crate::utils::hash::{hash_dir, hash_dir_cached, invalidate_hash_cache};
use crate::utils::path::{
  is_symlink, is_within, path_to_string, remove_path_any, same_path, validate_skill_name,
};
use crate::utils::time::{now_file_stamp, now_rfc3339};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const TRASH_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

pub fn list_hub_skills(env: &Env) -> Result<Vec<HubSkillView>, String> {
  migrate_skill_ids(env)?;
  adopt_untracked_hub_dirs(env)?;
  prune_stale_agent_ids(env)?;
  let lock = store(env).read()?;
  Ok(
    lock
      .skills
      .iter()
      .map(|(id, record)| build_view(env, id, record))
      .collect(),
  )
}

pub fn get_hub_skill(env: &Env, key: &str) -> Result<Option<HubSkillView>, String> {
  Ok(find_skill(env, key)?.map(|(id, record)| build_view(env, &id, &record)))
}

/// The skill a command argument names: an id, or a name that only one skill has.
pub fn find_skill(env: &Env, key: &str) -> Result<Option<(String, SkillRecord)>, String> {
  migrate_skill_ids(env)?;
  store(env).read()?.resolve(key)
}

/// Like [`find_skill`], but a skill that is not in the hub is an error.
pub fn resolve_skill(env: &Env, key: &str) -> Result<(String, SkillRecord), String> {
  find_skill(env, key)?.ok_or_else(|| format!("Skill '{}' is not in the hub", key.trim()))
}

/// Hub directories without a record become records with no source. Two forms are adopted:
/// `skills/<id>/<name>` (also what an interrupted import or migration leaves) and
/// `skills/<name>` (a directory added by hand), which moves to `skills/<new id>/<name>`.
fn adopt_untracked_hub_dirs(env: &Env) -> Result<(), String> {
  if !env.hub_root.is_dir() || untracked_hub_dirs(env)?.is_empty() {
    return Ok(());
  }
  let _ops = ops_guard_unless_held(&env.youskill_root)?;
  let mut adopted: Vec<(String, String, String)> = Vec::new();
  for (dir_name, name) in untracked_hub_dirs(env)? {
    let id = if is_skill_id(&dir_name) {
      dir_name
    } else {
      let id = new_skill_id();
      let old_dir = env.hub_root.join(&dir_name);
      move_dir(&old_dir, &env.hub_dir(&id, &name))?;
      invalidate_hash_cache(&old_dir);
      id
    };
    if let Ok(hash) = hash_dir_cached(&env.hub_dir(&id, &name)) {
      adopted.push((id, name, hash));
    }
  }
  let timestamp = now_rfc3339();
  store(env).update(|lock| {
    for (id, name, hash) in adopted {
      lock.skills.entry(id).or_insert(SkillRecord {
        name,
        source: SkillSource::None,
        hash,
        imported_at: timestamp.clone(),
        updated_at: timestamp.clone(),
        installs: Vec::new(),
      });
    }
    Ok(())
  })
}

/// `(hub directory name, skill name)` of every hub directory that has no record.
fn untracked_hub_dirs(env: &Env) -> Result<Vec<(String, String)>, String> {
  let known = store(env).read()?.skills;
  let mut untracked = Vec::new();
  for entry in fs::read_dir(&env.hub_root).map_err(|e| e.to_string())? {
    let entry = entry.map_err(|e| e.to_string())?;
    let dir = entry.path();
    let dir_name = entry.file_name().to_string_lossy().to_string();
    if !dir.is_dir() || known.contains_key(&dir_name) {
      continue;
    }
    let name = if is_skill_id(&dir_name) {
      skill_dir_in(&dir)
    } else if dir.join(SKILL_MD).is_file() {
      Some(dir_name.clone())
    } else {
      None
    };
    if let Some(name) = name.filter(|name| validate_skill_name(name).is_ok()) {
      untracked.push((dir_name, name));
    }
  }
  Ok(untracked)
}

/// Name of the one skill directory inside `skills/<id>/`.
pub fn skill_dir_in(id_dir: &Path) -> Option<String> {
  let mut names = fs::read_dir(id_dir)
    .ok()?
    .flatten()
    .filter(|entry| entry.path().join(SKILL_MD).is_file())
    .map(|entry| entry.file_name().to_string_lossy().to_string());
  let name = names.next()?;
  names.next().is_none().then_some(name)
}

/// Remove `skills/<id>/` once its skill directory is gone.
fn remove_id_dir(hub_dir: &Path) {
  if let Some(id_dir) = hub_dir.parent() {
    let _ = fs::remove_dir(id_dir);
  }
}

/// Drop agent ids from install records that no longer resolve to the record's directory.
/// An app that moved to the shared `~/.agents/skills` leaves its id behind on the old
/// directory, where it would claim an agent reads a folder it never looks at. Ids are only
/// removed while at least one recorded agent still resolves there.
fn prune_stale_agent_ids(env: &Env) -> Result<(), String> {
  let lock = store(env).read()?;
  let mut changes: Vec<(String, String, Vec<String>)> = Vec::new();
  for (id, record) in &lock.skills {
    for install in &record.installs {
      let Some(matched) = classify_agent_root(env, Path::new(&install.path)) else {
        continue;
      };
      let kept: Vec<String> = install
        .agent_ids
        .iter()
        .filter(|id| matched.agent_ids.contains(id))
        .cloned()
        .collect();
      if !kept.is_empty() && kept.len() != install.agent_ids.len() {
        changes.push((id.clone(), install.path.clone(), kept));
      }
    }
  }
  if changes.is_empty() {
    return Ok(());
  }

  let _ops = ops_guard(env)?;
  store(env).update(|lock| {
    for (id, path, kept) in &changes {
      let Some(record) = lock.skills.get_mut(id) else {
        continue;
      };
      if let Some(install) = record
        .installs
        .iter_mut()
        .find(|install| install.path == *path)
      {
        install.agent_ids = kept.clone();
      }
    }
    Ok(())
  })
}

/// Move a directory into `~/.youskill/.trash/<name>-<stamp>` so a replacement can be undone.
pub fn trash_dir(env: &Env, dir: &Path) -> Result<PathBuf, String> {
  let name = dir
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| "skill".to_string());
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
  if is_symlink(dir) {
    remove_path_any(dir)?;
    return Ok(dest);
  }
  move_dir(dir, &dest)?;
  invalidate_hash_cache(dir);
  Ok(dest)
}

pub fn sweep_trash(env: &Env) {
  let Ok(entries) = fs::read_dir(&env.trash_root) else {
    return;
  };
  let now = SystemTime::now();
  for entry in entries.flatten() {
    let Ok(meta) = entry.metadata() else {
      continue;
    };
    let Ok(modified) = meta.modified() else {
      continue;
    };
    if now.duration_since(modified).unwrap_or_default() > TRASH_MAX_AGE {
      let _ = remove_path_any(&entry.path());
    }
  }
}

/// Copy staged skill directories into the hub and record them. With `overwrite`, an
/// existing skill of the same name is replaced (its installs are kept and unmodified copy
/// targets are pushed afterwards).
pub fn import_skills(
  env: &Env,
  items: Vec<ImportItem>,
  overwrite: bool,
) -> Result<Vec<ImportOutcome>, String> {
  let _ops = ops_guard(env)?;
  migrate_skill_ids(env)?;
  let mut outcomes = Vec::new();
  let mut errors = Vec::new();
  let mut replaced_ids = Vec::new();

  for item in items {
    match import_one(env, &item, overwrite) {
      Ok(outcome) => {
        if outcome.replaced {
          replaced_ids.push(outcome.id.clone());
        }
        outcomes.push(outcome);
      },
      Err(err) => errors.push(format!("{}: {}", item.name, err)),
    }
  }

  for id in replaced_ids {
    if let Err(err) = push_targets(env, &id, None, false) {
      tracing::warn!("push after replacing '{}' failed: {}", id, err);
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

/// What each item does by default: replace the record of the same source, replace the
/// newest same-name zip skill, or add a new record. Callers show the plan and send their
/// choice back as `ImportItem::replace_id`.
pub fn plan_import(env: &Env, items: &[ImportItem]) -> Result<Vec<ImportPlan>, String> {
  migrate_skill_ids(env)?;
  let lock = store(env).read()?;
  Ok(
    items
      .iter()
      .map(|item| plan_one(env, &lock, item))
      .collect(),
  )
}

fn plan_one(env: &Env, lock: &LockFile, item: &ImportItem) -> ImportPlan {
  let name = item.name.trim();
  // A folder inside an agent root is that skill's install, so the record that owns the
  // install is the same skill.
  let install_path = match &item.source {
    SkillSource::Folder { path } if classify_agent_root(env, Path::new(path)).is_some() => {
      Some(Path::new(path))
    },
    _ => None,
  };
  let same_source = |record: &SkillRecord| {
    record.source.same_location(&item.source)
      || install_path.is_some_and(|path| {
        record
          .installs
          .iter()
          .any(|install| same_path(Path::new(&install.path), path))
      })
  };

  let mut matches: Vec<SkillMatch> = Vec::new();
  for (id, record) in &lock.skills {
    let same = same_source(record);
    if same || record.name.eq_ignore_ascii_case(name) {
      matches.push(SkillMatch {
        id: id.clone(),
        name: record.name.clone(),
        source_key: record.source.key(),
        same_source: same,
        updated_at: record.updated_at.clone(),
      });
    }
  }
  matches.sort_by(|a, b| {
    b.same_source
      .cmp(&a.same_source)
      .then_with(|| b.updated_at.cmp(&a.updated_at))
  });

  let replace_required = matches.first().is_some_and(|m| m.same_source);
  let default_replace_id = if replace_required {
    matches.first().map(|m| m.id.clone())
  } else if matches!(item.source, SkillSource::Zip { .. }) {
    // A zip import is usually a newer version of an earlier zip import.
    matches
      .iter()
      .find(|m| m.source_key == "zip" || m.source_key == "none")
      .map(|m| m.id.clone())
  } else {
    None
  };
  ImportPlan {
    name: name.to_string(),
    source_key: item.source.key(),
    matches,
    default_replace_id,
    replace_required,
  }
}

fn import_one(env: &Env, item: &ImportItem, overwrite: bool) -> Result<ImportOutcome, String> {
  let name = item.name.trim().to_string();
  validate_skill_name(&name)?;
  let src = Path::new(&item.tmp_path);
  if !src.is_dir() || !src.join(SKILL_MD).is_file() {
    return Err(format!(
      "Staged skill directory is invalid: {}",
      src.to_string_lossy()
    ));
  }

  let store = store(env);
  let lock = store.read()?;
  let plan = plan_one(env, &lock, item);
  let existing_id = if plan.replace_required {
    let same = plan.default_replace_id.clone();
    if item.replace_id.is_some() && item.replace_id != same {
      return Err(format!(
        "{} is already in the hub as {}",
        plan.source_key,
        same.unwrap_or_default()
      ));
    }
    same
  } else {
    item.replace_id.clone()
  };
  if let Some(id) = &existing_id {
    let record = lock
      .skills
      .get(id)
      .ok_or_else(|| format!("Skill '{}' is not in the hub", id))?;
    if !overwrite {
      return Err(format!(
        "Skill already exists in the hub: {} ({})",
        id,
        record.source.key()
      ));
    }
    if record.name != name {
      return Err(format!(
        "Skill '{}' cannot replace '{}': the names differ",
        name, record.name
      ));
    }
  }

  // A folder that is itself an agent target is not a source: register it as an install.
  let mut source = item.source.clone();
  let mut auto_install: Option<(InstallScope, Option<String>, String, Vec<String>)> = None;
  if let SkillSource::Folder { path } = &source {
    let folder = Path::new(path);
    if is_within(folder, &env.youskill_root) {
      source = SkillSource::None;
    } else if let Some(matched) = classify_agent_root(env, folder) {
      auto_install = Some((
        matched.scope,
        matched.project_path,
        path_to_string(folder),
        matched.agent_ids,
      ));
      source = SkillSource::None;
    }
  }

  let replaced = existing_id.is_some();
  let id = existing_id.unwrap_or_else(new_skill_id);
  let hub_dir = env.hub_dir(&id, &name);
  if hub_dir.exists() || is_symlink(&hub_dir) {
    trash_dir(env, &hub_dir)?;
  }
  copy_dir(src, &hub_dir, &CopyOpts::skill())?;
  invalidate_hash_cache(&hub_dir);
  let hash = hash_dir(&hub_dir)?;
  if is_staged_temp_path(src) {
    let _ = remove_path_any(src);
  }

  let timestamp = now_rfc3339();
  store.update(|lock| {
    let record = lock.skills.entry(id.clone()).or_insert(SkillRecord {
      name: name.clone(),
      source: SkillSource::None,
      hash: hash.clone(),
      imported_at: timestamp.clone(),
      updated_at: timestamp.clone(),
      installs: Vec::new(),
    });
    record.source = source.clone();
    record.hash = hash.clone();
    record.updated_at = timestamp.clone();
    if let Some((scope, project_path, path, agent_ids)) = &auto_install {
      if let Some(install) = record.installs.iter_mut().find(|i| &i.path == path) {
        install.hash = hash.clone();
        for id in agent_ids {
          if !install.agent_ids.contains(id) {
            install.agent_ids.push(id.clone());
          }
        }
      } else {
        record.installs.push(InstallRecord {
          scope: *scope,
          project_path: project_path.clone(),
          path: path.clone(),
          mode: InstallMode::Copy,
          hash: hash.clone(),
          installed_at: timestamp.clone(),
          agent_ids: agent_ids.clone(),
        });
      }
    }
    Ok(())
  })?;

  Ok(ImportOutcome {
    id,
    name,
    hub_path: path_to_string(&hub_dir),
    hash,
    replaced,
  })
}

/// Replace the hub copy with fresh content from the source (already downloaded / located
/// at `src_dir`). Refuses without `force` when the hub has local edits.
pub fn pull_from_dir(
  env: &Env,
  key: &str,
  src_dir: &Path,
  updated_source: Option<SkillSource>,
  force: bool,
) -> Result<ActionResult, String> {
  let _ops = ops_guard(env)?;
  let (id, record) = resolve_skill(env, key)?;
  let store = store(env);
  if let Some(source) = updated_source.as_ref().filter(|s| s.has_location()) {
    let lock = store.read()?;
    if let Some((other, _)) = lock
      .skills
      .iter()
      .find(|(other, r)| **other != id && r.source.same_location(source))
    {
      return Err(format!(
        "{} is already in the hub as {}",
        source.key(),
        other
      ));
    }
  }
  if !src_dir.is_dir() || !src_dir.join(SKILL_MD).is_file() {
    return Err(format!(
      "Source directory is invalid: {}",
      src_dir.to_string_lossy()
    ));
  }
  let probe = probe_hub(env, &id, &record);
  if matches!(probe.state, HubState::Modified | HubState::NameMismatch) && !force {
    return Ok(ActionResult::blocked(
      vec![format!("Hub copy of '{}' has local changes", record.name)],
      Some(build_view(env, &id, &record)),
    ));
  }

  let hub_dir = env.hub_dir(&id, &record.name);
  if hub_dir.exists() || is_symlink(&hub_dir) {
    trash_dir(env, &hub_dir)?;
  }
  copy_dir(src_dir, &hub_dir, &CopyOpts::skill())?;
  invalidate_hash_cache(&hub_dir);
  let hash = hash_dir(&hub_dir)?;
  if is_staged_temp_path(src_dir) {
    let _ = remove_path_any(src_dir);
  }
  let timestamp = now_rfc3339();
  store.update(|lock| {
    let record = lock
      .skills
      .get_mut(&id)
      .ok_or_else(|| format!("Skill '{}' is not in the hub", key))?;
    record.hash = hash.clone();
    record.updated_at = timestamp.clone();
    if let Some(source) = &updated_source {
      record.source = source.clone();
    }
    Ok(())
  })?;
  drop(store);
  push_targets(env, &id, None, false)
}

/// Mirror the hub copy back into a `folder` source.
pub fn push_source(env: &Env, key: &str, force: bool) -> Result<ActionResult, String> {
  let _ops = ops_guard(env)?;
  let (id, record) = resolve_skill(env, key)?;
  let SkillSource::Folder { path } = &record.source else {
    return Err("Only folder sources can be pushed to".to_string());
  };
  let source_dir = Path::new(path);
  if !source_dir.is_dir() {
    return Err(format!("Source folder is missing: {}", path));
  }
  let hub_dir = env.hub_dir(&id, &record.name);
  if !hub_dir.is_dir() {
    return Err(format!(
      "Hub copy is missing: {}",
      hub_dir.to_string_lossy()
    ));
  }
  let source_hash = hash_dir(source_dir)?;
  if source_hash != record.hash && !force {
    return Ok(ActionResult::blocked(
      vec![format!("Source folder {} has changes not in the hub", path)],
      Some(build_view(env, &id, &record)),
    ));
  }
  replace_dir_atomic(&hub_dir, source_dir, &CopyOpts::skill(), &[".git"])?;
  invalidate_hash_cache(source_dir);
  Ok(ActionResult::applied(build_view(env, &id, &record)))
}

/// Accept whatever is in the hub directory as the current version.
pub fn accept_hub(env: &Env, key: &str) -> Result<ActionResult, String> {
  let _ops = ops_guard(env)?;
  let (id, record) = resolve_skill(env, key)?;
  let store = store(env);
  let hub_dir = env.hub_dir(&id, &record.name);
  if !hub_dir.is_dir() {
    return Err(format!(
      "Hub copy is missing: {}",
      hub_dir.to_string_lossy()
    ));
  }
  invalidate_hash_cache(&hub_dir);
  let hash = hash_dir(&hub_dir)?;
  let timestamp = now_rfc3339();
  let record = store.update(|lock| {
    let record = lock
      .skills
      .get_mut(&id)
      .ok_or_else(|| format!("Skill '{}' is not in the hub", key))?;
    record.hash = hash.clone();
    record.updated_at = timestamp.clone();
    Ok(record.clone())
  })?;
  Ok(ActionResult::applied(build_view(env, &id, &record)))
}

/// Remove a skill from the hub. With `remove_installs = false`, symlink targets are turned
/// into real copies first so agents keep working. Runs inside a caller's operation (a
/// library restore) without taking the lock again.
pub fn remove_hub_skill(env: &Env, key: &str, remove_installs: bool) -> Result<(), String> {
  let _ops = ops_guard_unless_held(&env.youskill_root)?;
  let (id, record) = resolve_skill(env, key)?;
  let store = store(env);
  let hub_dir = env.hub_dir(&id, &record.name);

  let mut errors = Vec::new();
  for install in &record.installs {
    let target = Path::new(&install.path);
    if remove_installs {
      if target.exists() || is_symlink(target) {
        if let Err(err) = remove_path_any(target) {
          errors.push(format!("{}: {}", install.path, err));
        } else {
          invalidate_hash_cache(target);
        }
      }
    } else if let Err(err) = materialize_symlink(&hub_dir, install) {
      errors.push(format!("{}: {}", install.path, err));
    }
  }

  if hub_dir.exists() || is_symlink(&hub_dir) {
    trash_dir(env, &hub_dir)?;
  }
  remove_id_dir(&hub_dir);
  store.update(|lock| {
    lock.skills.remove(&id);
    Ok(())
  })?;
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  Ok(())
}

/// Offline sync actions. `PullSource` for GitHub sources needs a download and is handled by
/// the command layer, which then calls `pull_from_dir`.
pub fn sync_skill(env: &Env, key: &str, action: SyncAction) -> Result<ActionResult, String> {
  let (id, record) = resolve_skill(env, key)?;
  match action {
    SyncAction::PullSource { force } => match &record.source {
      SkillSource::Folder { path } => pull_from_dir(env, &id, Path::new(path), None, force),
      SkillSource::Github { .. } => {
        Err("GitHub sources must be pulled through the async command".to_string())
      },
      SkillSource::Zip { .. } | SkillSource::None => {
        Err("This skill has no source to pull from".to_string())
      },
    },
    SyncAction::PushTargets { targets, force } => {
      let _ops = ops_guard(env)?;
      push_targets(env, &id, targets.as_deref(), force)
    },
    SyncAction::AdoptTarget { path, force } => {
      let _ops = ops_guard(env)?;
      adopt_target(env, &id, &path, force)
    },
    SyncAction::AcceptHub => accept_hub(env, &id),
    SyncAction::PushSource { force } => push_source(env, &id, force),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::{AgentApp, InstallRequest, InstallTargetSpec, TargetState};
  use crate::services::install_service::install_skill;
  use std::fs;

  fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
  }

  fn env(tmp: &Path) -> Env {
    Env::for_test(
      tmp,
      vec![AgentApp {
        id: "claude-code".to_string(),
        display_name: "Claude Code".to_string(),
        project_path: Some(".claude/skills".to_string()),
        global_path: Some("~/.claude/skills".to_string()),
        detect_path: None,
        profile_path: Some("AGENTS.md".to_string()),
        global_profile_path: None,
        is_user_custom: false,
      }],
      vec![],
    )
  }

  fn staged(tmp: &Path, name: &str, body: &str) -> ImportItem {
    let dir = tmp.join("staged").join(name);
    write(&dir, "SKILL.md", &format!("---\nname: {name}\n---\n{body}"));
    ImportItem {
      name: name.to_string(),
      tmp_path: dir.to_string_lossy().to_string(),
      source: SkillSource::None,
      replace_id: None,
    }
  }

  fn staged_from(tmp: &Path, name: &str, body: &str, source: SkillSource) -> ImportItem {
    ImportItem {
      source,
      ..staged(tmp, name, body)
    }
  }

  fn github(repo: &str, branch: Option<&str>) -> SkillSource {
    SkillSource::Github {
      repo: repo.to_string(),
      url: format!("https://github.com/{repo}.git"),
      skill_path: "skills/tdd/SKILL.md".to_string(),
      branch: branch.map(str::to_string),
      remote_sha: None,
      marketplace_id: None,
      latest_remote_sha: None,
      checked_at: None,
    }
  }

  fn zip(path: &str) -> SkillSource {
    SkillSource::Zip {
      path: path.to_string(),
    }
  }

  fn import(env: &Env, item: ImportItem, overwrite: bool) -> Result<ImportOutcome, String> {
    import_skills(env, vec![item], overwrite).map(|mut outcomes| outcomes.remove(0))
  }

  #[test]
  fn listing_prunes_agent_ids_that_no_longer_read_the_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let mut env = env(tmp.path());
    // Two apps that both read the shared user directory, plus a legacy directory that only
    // one of them ever used.
    env.agent_apps = vec![
      AgentApp {
        id: "agents".to_string(),
        display_name: "Agents".to_string(),
        project_path: Some(".agents/skills".to_string()),
        global_path: Some("~/.agents/skills".to_string()),
        detect_path: None,
        profile_path: Some("AGENTS.md".to_string()),
        global_profile_path: None,
        is_user_custom: false,
      },
      AgentApp {
        id: "codex".to_string(),
        display_name: "Codex".to_string(),
        project_path: Some(".agents/skills".to_string()),
        global_path: Some("~/.agents/skills".to_string()),
        detect_path: None,
        profile_path: Some("AGENTS.md".to_string()),
        global_profile_path: None,
        is_user_custom: false,
      },
    ];
    let id = import_skills(&env, vec![staged(tmp.path(), "foo", "v1")], false).unwrap()[0]
      .id
      .clone();

    let legacy = env.home.join(".codex/skills/foo");
    write(&legacy, "SKILL.md", "---\nname: foo\n---\nv1");
    let legacy_path = legacy.to_string_lossy().to_string();
    store(&env)
      .update(|lock| {
        let record = lock.skills.get_mut(&id).unwrap();
        record.installs.push(InstallRecord {
          scope: InstallScope::User,
          project_path: None,
          path: legacy_path.clone(),
          mode: InstallMode::Copy,
          hash: record.hash.clone(),
          installed_at: now_rfc3339(),
          agent_ids: vec!["codex".to_string(), "agents".to_string()],
        });
        Ok(())
      })
      .unwrap();

    list_hub_skills(&env).unwrap();

    let record = store(&env).get(&id).unwrap().unwrap();
    let install = record
      .installs
      .iter()
      .find(|install| install.path == legacy_path)
      .unwrap();
    // `~/.codex/skills` is codex's legacy root; agents never read it.
    assert_eq!(install.agent_ids, vec!["codex".to_string()]);
  }

  #[test]
  fn import_new_conflict_and_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let outcomes = import_skills(&env, vec![staged(tmp.path(), "foo", "v1")], false).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].replaced);
    assert!(env.hub_dir_of("foo").join("SKILL.md").is_file());
    let first_hash = outcomes[0].hash.clone();

    // Replacing needs overwrite, and a replacement keeps the name.
    let first_id = outcomes[0].id.clone();
    let replace = |name: &str| ImportItem {
      replace_id: Some(first_id.clone()),
      ..staged(tmp.path(), name, "v2")
    };
    assert!(import(&env, replace("foo"), false).is_err());
    assert!(import(&env, replace("FOO"), true).is_err());

    // Install, then overwrite: installs are kept and the unmodified copy is pushed.
    install_skill(
      &env,
      InstallRequest {
        name: "foo".to_string(),
        targets: vec![InstallTargetSpec {
          scope: InstallScope::User,
          project_path: None,
          agent_id: "claude-code".to_string(),
        }],
        mode: Some(InstallMode::Copy),
        force: false,
      },
    )
    .unwrap();
    let outcome = import(&env, replace("foo"), true).unwrap();
    assert!(outcome.replaced);
    assert_eq!(outcome.id, first_id);
    assert_ne!(outcome.hash, first_hash);
    let view = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(view.installs.len(), 1);
    assert_eq!(view.installs[0].state, TargetState::InSync);
    assert!(
      fs::read_to_string(env.home.join(".claude/skills/foo/SKILL.md"))
        .unwrap()
        .ends_with("v2")
    );
    assert!(env.trash_root.read_dir().unwrap().next().is_some());
  }

  #[test]
  fn same_name_from_two_sources_gives_two_skills() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let a = import(
      &env,
      staged_from(
        tmp.path(),
        "tdd",
        "A",
        github("mattpocock/skills", Some("main")),
      ),
      false,
    )
    .unwrap();
    let b = import(
      &env,
      staged_from(
        tmp.path(),
        "tdd",
        "B",
        github("backnotprop/pstack", Some("main")),
      ),
      false,
    )
    .unwrap();
    assert_ne!(a.id, b.id);
    assert!(!b.replaced);
    assert!(env.hub_dir(&a.id, "tdd").join("SKILL.md").is_file());
    assert!(env.hub_dir(&b.id, "tdd").join("SKILL.md").is_file());
    assert!(
      get_hub_skill(&env, "tdd").is_err(),
      "the name alone is ambiguous"
    );
    assert_eq!(
      get_hub_skill(&env, &b.id).unwrap().unwrap().source_key,
      "github:backnotprop/pstack@main:skills/tdd"
    );

    // Another branch of the same skill is another skill.
    let next = import(
      &env,
      staged_from(
        tmp.path(),
        "tdd",
        "N",
        github("mattpocock/skills", Some("next")),
      ),
      false,
    )
    .unwrap();
    assert_ne!(next.id, a.id);
    assert_eq!(store(&env).read().unwrap().skills.len(), 3);
  }

  #[test]
  fn known_source_replaces_only_with_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let source = github("o/r", Some("main"));
    let first = import(
      &env,
      staged_from(tmp.path(), "tdd", "v1", source.clone()),
      false,
    )
    .unwrap();

    let again = staged_from(tmp.path(), "tdd", "v2", source.clone());
    let plan = plan_import(&env, std::slice::from_ref(&again)).unwrap();
    assert!(plan[0].replace_required);
    assert_eq!(
      plan[0].default_replace_id.as_deref(),
      Some(first.id.as_str())
    );
    let err = import(&env, again.clone(), false).unwrap_err();
    assert!(err.contains("already exists"), "{err}");

    // A source in the hub cannot be added again, also not as a replacement of another skill.
    let other = import(&env, staged(tmp.path(), "tdd", "x"), false).unwrap();
    let wrong = ImportItem {
      replace_id: Some(other.id.clone()),
      ..again.clone()
    };
    assert!(import(&env, wrong, true).is_err());

    let replaced = import(&env, again, true).unwrap();
    assert!(replaced.replaced);
    assert_eq!(replaced.id, first.id);
    assert_eq!(store(&env).read().unwrap().skills.len(), 2);
  }

  #[test]
  fn import_with_a_branch_matches_a_legacy_record_without_one() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let legacy = import(
      &env,
      staged_from(tmp.path(), "tdd", "v1", github("o/r", None)),
      false,
    )
    .unwrap();
    let item = staged_from(tmp.path(), "tdd", "v2", github("o/r", Some("main")));
    let outcome = import(&env, item, true).unwrap();
    assert_eq!(outcome.id, legacy.id);
    assert_eq!(
      get_hub_skill(&env, &legacy.id).unwrap().unwrap().source_key,
      "github:o/r@main:skills/tdd"
    );
  }

  #[test]
  fn zip_import_replaces_the_newest_zip_skill_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let old = import(
      &env,
      staged_from(tmp.path(), "tdd", "1", zip("/a.zip")),
      false,
    )
    .unwrap();
    let newer = import(
      &env,
      staged_from(tmp.path(), "tdd", "2", zip("/b.zip")),
      false,
    )
    .unwrap();
    assert_ne!(old.id, newer.id, "no replace_id adds a record");
    import(
      &env,
      staged_from(tmp.path(), "tdd", "gh", github("o/r", Some("main"))),
      false,
    )
    .unwrap();

    let item = staged_from(tmp.path(), "tdd", "3", zip("/c.zip"));
    let plan = plan_import(&env, std::slice::from_ref(&item)).unwrap();
    assert!(!plan[0].replace_required);
    assert_eq!(plan[0].matches.len(), 3);
    assert_eq!(
      plan[0].default_replace_id.as_deref(),
      Some(newer.id.as_str())
    );

    let chosen = ImportItem {
      replace_id: plan[0].default_replace_id.clone(),
      ..item
    };
    assert!(import(&env, chosen.clone(), false).is_err());
    let outcome = import(&env, chosen, true).unwrap();
    assert_eq!(outcome.id, newer.id);
    assert!(outcome.replaced);
  }

  #[test]
  fn zip_import_does_not_replace_a_github_skill_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    import(
      &env,
      staged_from(tmp.path(), "tdd", "gh", github("o/r", Some("main"))),
      false,
    )
    .unwrap();
    let item = staged_from(tmp.path(), "tdd", "z", zip("/a.zip"));
    let plan = plan_import(&env, std::slice::from_ref(&item)).unwrap();
    assert_eq!(plan[0].matches.len(), 1);
    assert!(!plan[0].matches[0].same_source);
    assert!(plan[0].default_replace_id.is_none());
  }

  #[test]
  fn pull_refuses_a_source_that_another_skill_has() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    import(
      &env,
      staged_from(tmp.path(), "tdd", "a", github("o/a", Some("main"))),
      false,
    )
    .unwrap();
    let b = import(
      &env,
      staged_from(tmp.path(), "tdd", "b", github("o/b", Some("main"))),
      false,
    )
    .unwrap();
    let src = tmp.path().join("pulled");
    write(&src, "SKILL.md", "---\nname: tdd\n---\nnew");
    let err =
      pull_from_dir(&env, &b.id, &src, Some(github("o/a", Some("main"))), false).unwrap_err();
    assert!(err.contains("already in the hub"), "{err}");
  }

  #[test]
  fn import_rejects_invalid_names_and_missing_skill_md() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let mut item = staged(tmp.path(), "bad", "x");
    item.name = "../bad".to_string();
    assert!(import_skills(&env, vec![item], false).is_err());
    let empty = tmp.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let item = ImportItem {
      name: "empty".to_string(),
      tmp_path: empty.to_string_lossy().to_string(),
      source: SkillSource::None,
      replace_id: None,
    };
    assert!(import_skills(&env, vec![item], false).is_err());
  }

  #[test]
  fn folder_inside_agent_root_becomes_install_not_source() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let dir = env.home.join(".claude/skills/foo");
    write(&dir, "SKILL.md", "---\nname: foo\n---\n");
    let item = ImportItem {
      name: "foo".to_string(),
      tmp_path: dir.to_string_lossy().to_string(),
      source: SkillSource::Folder {
        path: dir.to_string_lossy().to_string(),
      },
      replace_id: None,
    };
    import_skills(&env, vec![item], false).unwrap();
    let view = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(view.source, SkillSource::None);
    assert_eq!(view.installs.len(), 1);
    assert_eq!(view.installs[0].record.agent_ids, vec!["claude-code"]);
    assert_eq!(view.installs[0].state, TargetState::InSync);
    assert!(dir.is_dir(), "source dir outside temp must not be removed");
  }

  #[test]
  fn folder_source_pull_and_push() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let src = tmp.path().join("mysrc");
    write(&src, "SKILL.md", "---\nname: foo\n---\nv1");
    let item = ImportItem {
      name: "foo".to_string(),
      tmp_path: src.to_string_lossy().to_string(),
      source: SkillSource::Folder {
        path: src.to_string_lossy().to_string(),
      },
      replace_id: None,
    };
    import_skills(&env, vec![item], false).unwrap();

    write(&src, "SKILL.md", "---\nname: foo\n---\nv2");
    let view = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(
      view.source_state,
      crate::models::SourceState::SourceModified
    );
    let pulled = sync_skill(&env, "foo", SyncAction::PullSource { force: false }).unwrap();
    assert!(pulled.applied);
    assert!(fs::read_to_string(env.hub_dir_of("foo").join("SKILL.md"))
      .unwrap()
      .ends_with("v2"));

    // Edit hub -> pull is blocked; accept -> push back to source.
    write(
      &env.hub_dir_of("foo"),
      "SKILL.md",
      "---\nname: foo\n---\nv3",
    );
    write(&src, "SKILL.md", "---\nname: foo\n---\nv2b");
    let blocked = sync_skill(&env, "foo", SyncAction::PullSource { force: false }).unwrap();
    assert!(!blocked.applied);
    accept_hub(&env, "foo").unwrap();
    let blocked = sync_skill(&env, "foo", SyncAction::PushSource { force: false }).unwrap();
    assert!(!blocked.applied, "source changed too");
    let pushed = sync_skill(&env, "foo", SyncAction::PushSource { force: true }).unwrap();
    assert!(pushed.applied);
    assert!(fs::read_to_string(src.join("SKILL.md"))
      .unwrap()
      .ends_with("v3"));
  }

  #[cfg(unix)]
  #[test]
  fn remove_hub_skill_materializes_or_removes_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    import_skills(&env, vec![staged(tmp.path(), "foo", "v1")], false).unwrap();
    install_skill(
      &env,
      InstallRequest {
        name: "foo".to_string(),
        targets: vec![InstallTargetSpec {
          scope: InstallScope::User,
          project_path: None,
          agent_id: "claude-code".to_string(),
        }],
        mode: Some(InstallMode::Symlink),
        force: false,
      },
    )
    .unwrap();
    let target = env.home.join(".claude/skills/foo");
    assert!(is_symlink(&target));

    let hub_dir = env.hub_dir_of("foo");
    remove_hub_skill(&env, "foo", false).unwrap();
    assert!(!is_symlink(&target));
    assert!(target.join("SKILL.md").is_file());
    assert!(!hub_dir.exists());
    assert!(!hub_dir.parent().unwrap().exists());
    assert!(get_hub_skill(&env, "foo").unwrap().is_none());

    import_skills(&env, vec![staged(tmp.path(), "bar", "v1")], false).unwrap();
    install_skill(
      &env,
      InstallRequest {
        name: "bar".to_string(),
        targets: vec![InstallTargetSpec {
          scope: InstallScope::User,
          project_path: None,
          agent_id: "claude-code".to_string(),
        }],
        mode: Some(InstallMode::Copy),
        force: false,
      },
    )
    .unwrap();
    remove_hub_skill(&env, "bar", true).unwrap();
    assert!(!env.home.join(".claude/skills/bar").exists());
  }

  #[test]
  fn untracked_id_dir_is_adopted_with_its_id() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    let id = new_skill_id();
    write(
      &env.hub_dir(&id, "kept"),
      "SKILL.md",
      "---\nname: kept\n---\n",
    );
    let list = list_hub_skills(&env).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].name, "kept");
    assert_eq!(list[0].hub_state, HubState::Ok);
  }

  #[test]
  fn untracked_hub_dirs_are_adopted_on_list() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path());
    write(
      &env.hub_root.join("manual"),
      "SKILL.md",
      "---\nname: manual\n---\n",
    );
    let list = list_hub_skills(&env).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "manual");
    assert_eq!(list[0].hub_state, HubState::Ok);
    assert!(!list[0].has_drift);
    // The directory moves to `skills/<id>/manual`.
    assert!(is_skill_id(&list[0].id));
    assert!(!env.hub_root.join("manual").exists());
    assert!(env
      .hub_dir(&list[0].id, "manual")
      .join("SKILL.md")
      .is_file());
  }
}
