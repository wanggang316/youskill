//! One-time, idempotent migration from the pre-hub layout:
//! `~/.agents/skills` (canonical copies), `~/.agents/.skill-lock.json` (GitHub lock),
//! `<config_dir>/youskill/native-skill-lock.json`, `<project>/skills-lock.json`, plus every
//! agent's own skills directory. Nothing legacy is deleted.
//!
//! Also the move from lock version 1 (records keyed by name, `skills/<name>`) to version 2
//! (records keyed by id, `skills/<id>/<name>`), see [`migrate_skill_ids`].

use crate::models::{
  is_skill_id, new_skill_id, InstallMode, InstallScope, MigrationReport, SkillRecord, SkillSource,
};
use crate::services::agent_apps_service::LEGACY_USER_ROOTS;
use crate::services::env::Env;
use crate::services::hub_service::skill_dir_in;
use crate::services::install_service::{register_existing_install, write_target};
use crate::services::lock_service::{ops_guard, ops_guard_unless_held, store};
use crate::utils::folder::{copy_dir, move_dir, read_skill_name, CopyOpts, SKILL_MD};
use crate::utils::hash::{hash_dir, invalidate_hash_cache};
use crate::utils::path::{
  expand_home_with, is_symlink, is_within, legacy_agents_root, same_path, symlink_points_to,
  validate_skill_name,
};
use crate::utils::time::now_rfc3339;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static CHECKED_THIS_PROCESS: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Legacy lock formats (read-only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyGithubEntry {
  #[serde(default)]
  pub source: String,
  #[serde(default)]
  pub source_url: String,
  #[serde(default)]
  pub skill_path: Option<String>,
  #[serde(default)]
  pub skill_folder_hash: Option<String>,
  #[serde(default)]
  pub installed_at: Option<String>,
}

impl LegacyGithubEntry {
  pub fn to_source(&self) -> SkillSource {
    let repo = if self.source.trim().is_empty() {
      crate::utils::github::GithubHelper::parse_github_url(&self.source_url)
        .map(|(owner, repo)| format!("{}/{}", owner, repo))
        .unwrap_or_default()
    } else {
      self.source.trim().to_string()
    };
    if repo.is_empty() {
      return SkillSource::None;
    }
    let url = if self.source_url.trim().is_empty() {
      format!("https://github.com/{}.git", repo)
    } else {
      self.source_url.trim().to_string()
    };
    SkillSource::Github {
      repo,
      url,
      skill_path: self.skill_path.clone().unwrap_or_default(),
      branch: Some("main".to_string()),
      remote_sha: self.skill_folder_hash.clone().filter(|s| !s.is_empty()),
      marketplace_id: None,
      latest_remote_sha: None,
      checked_at: None,
    }
  }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LegacyGithubLock {
  #[serde(default)]
  pub skills: HashMap<String, LegacyGithubEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyNativeEntry {
  #[serde(default)]
  pub installed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LegacyNativeLock {
  #[serde(default)]
  pub skills: HashMap<String, LegacyNativeEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyProjectEntry {
  #[serde(default)]
  pub source: String,
  #[serde(default)]
  pub source_type: String,
}

impl LegacyProjectEntry {
  pub fn to_source(&self) -> Option<SkillSource> {
    if self.source_type != "github" || self.source.trim().is_empty() {
      return None;
    }
    let repo = self.source.trim().to_string();
    Some(SkillSource::Github {
      url: format!("https://github.com/{}.git", repo),
      repo,
      skill_path: String::new(),
      branch: Some("main".to_string()),
      remote_sha: None,
      marketplace_id: None,
      latest_remote_sha: None,
      checked_at: None,
    })
  }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LegacyProjectLock {
  #[serde(default)]
  pub skills: HashMap<String, LegacyProjectEntry>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
  let content = fs::read_to_string(path).ok()?;
  serde_json::from_str(&content).ok()
}

pub fn read_legacy_github_lock(path: &Path) -> Option<LegacyGithubLock> {
  read_json(path)
}

pub fn read_legacy_native_lock(path: &Path) -> Option<LegacyNativeLock> {
  read_json(path)
}

pub fn read_legacy_project_lock(path: &Path) -> Option<LegacyProjectLock> {
  read_json(path)
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

pub fn needs_migration(env: &Env) -> Result<bool, String> {
  let store = store(env);
  if !store.exists() {
    return Ok(true);
  }
  let lock = store.read()?;
  Ok(match lock.migration {
    Some(report) => report.completed_at.is_none() || !report.errors.is_empty(),
    None => true,
  })
}

/// Run the migration at most once per process when it is still pending.
pub fn ensure_migrated(env: &Env) -> Result<Option<MigrationReport>, String> {
  if CHECKED_THIS_PROCESS.swap(true, Ordering::SeqCst) {
    return Ok(None);
  }
  if !needs_migration(env)? {
    return Ok(None);
  }
  migrate_legacy(env).map(Some)
}

pub fn migrate_legacy(env: &Env) -> Result<MigrationReport, String> {
  let _ops = ops_guard(env)?;
  migrate_skill_ids(env)?;
  let mut report = MigrationReport::default();
  let legacy_root = legacy_agents_root(&env.home);
  let github_lock =
    read_legacy_github_lock(&env.home.join(".agents").join(".skill-lock.json")).unwrap_or_default();
  let native_lock =
    read_legacy_native_lock(&env.config_dir.join("native-skill-lock.json")).unwrap_or_default();

  fs::create_dir_all(&env.hub_root).map_err(|e| e.to_string())?;

  // 1. Legacy canonical copies become hub copies (preferred content).
  if legacy_root.is_dir() {
    migrate_root(
      env,
      &legacy_root,
      InstallScope::User,
      None,
      None,
      &mut report,
      |name, dir_name| {
        github_lock
          .skills
          .get(name)
          .or_else(|| github_lock.skills.get(dir_name))
          .map(|entry| (entry.to_source(), entry.installed_at.clone()))
          .or_else(|| {
            native_lock
              .skills
              .get(name)
              .map(|entry| (SkillSource::None, entry.installed_at.clone()))
          })
      },
    );
  }

  // 2. Every user-level agent root, plus the app-specific directories apps used before
  //    adopting the shared `~/.agents/skills`.
  let mut visited_roots: Vec<PathBuf> = vec![legacy_root.clone()];
  for app in &env.agent_apps {
    let Ok(root) = env.agent_root(app, InstallScope::User, None) else {
      continue;
    };
    if !root.is_dir() || visited_roots.iter().any(|seen| same_path(seen, &root)) {
      continue;
    }
    visited_roots.push(root.clone());
    migrate_root(
      env,
      &root,
      InstallScope::User,
      None,
      None,
      &mut report,
      |_, _| None,
    );
  }
  for (agent_id, legacy_path) in LEGACY_USER_ROOTS {
    if env.agent(agent_id).is_none() {
      continue;
    }
    let root = expand_home_with(legacy_path, &env.home);
    if !root.is_dir() || visited_roots.iter().any(|seen| same_path(seen, &root)) {
      continue;
    }
    visited_roots.push(root.clone());
    migrate_root(
      env,
      &root,
      InstallScope::User,
      None,
      Some(vec![agent_id.to_string()]),
      &mut report,
      |_, _| None,
    );
  }

  // 3. Registered projects: every agent's project root, with the project lock as hint.
  for project in &env.projects {
    let project_root = PathBuf::from(&project.path);
    if !project_root.is_dir() {
      continue;
    }
    let project_lock =
      read_legacy_project_lock(&project_root.join("skills-lock.json")).unwrap_or_default();
    let mut seen_roots: Vec<PathBuf> = Vec::new();
    for app in &env.agent_apps {
      let Ok(root) = env.agent_root(app, InstallScope::Project, Some(&project_root)) else {
        continue;
      };
      if !root.is_dir() || seen_roots.iter().any(|seen| same_path(seen, &root)) {
        continue;
      }
      seen_roots.push(root.clone());
      migrate_root(
        env,
        &root,
        InstallScope::Project,
        Some(project.path.clone()),
        None,
        &mut report,
        |name, dir_name| {
          project_lock
            .skills
            .get(name)
            .or_else(|| project_lock.skills.get(dir_name))
            .and_then(|entry| entry.to_source())
            .map(|source| (source, None))
        },
      );
    }
  }

  report.completed_at = Some(now_rfc3339());
  let saved = report.clone();
  store(env).update(|lock| {
    lock.migration = Some(saved);
    Ok(())
  })?;
  Ok(report)
}

/// Walk one skills directory: import unknown skills into the hub, then record the
/// directory (or relinked symlink) as an install target.
fn migrate_root<F>(
  env: &Env,
  root: &Path,
  scope: InstallScope,
  project_path: Option<String>,
  agent_ids_override: Option<Vec<String>>,
  report: &mut MigrationReport,
  source_hint: F,
) where
  F: Fn(&str, &str) -> Option<(SkillSource, Option<String>)>,
{
  let Ok(entries) = fs::read_dir(root) else {
    return;
  };
  let legacy_root = legacy_agents_root(&env.home);
  let agent_ids: Vec<String> = agent_ids_override.unwrap_or_else(|| {
    env
      .agent_apps
      .iter()
      .filter(|app| {
        env
          .agent_root(app, scope, project_path.as_deref().map(Path::new))
          .map(|r| same_path(&r, root))
          .unwrap_or(false)
      })
      .map(|app| app.id.clone())
      .collect()
  });

  let mut dirs: Vec<PathBuf> = entries
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| path.is_dir() && path.join(SKILL_MD).is_file())
    .collect();
  dirs.sort();

  if is_within(root, &env.youskill_root) {
    return;
  }

  for dir in dirs {
    let dir_name = dir
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_default();
    let Some(name) = read_skill_name(&dir) else {
      continue;
    };
    if validate_skill_name(&name).is_err() {
      report.errors.push(format!(
        "{}: invalid skill name '{}'",
        dir.to_string_lossy(),
        name
      ));
      continue;
    }

    let resolved = is_symlink(&dir)
      .then(|| dir.canonicalize().ok())
      .flatten()
      .unwrap_or_default();
    let linked_to_hub = is_symlink(&dir) && is_within(&resolved, &env.hub_root);
    let linked_to_legacy = !linked_to_hub && is_symlink(&dir) && is_within(&resolved, &legacy_root);

    // Import into the hub when unknown.
    let known = match store(env).read() {
      Ok(lock) => lock.find_by_name(&name).map(|(id, _)| id.clone()),
      Err(err) => {
        report.errors.push(err);
        continue;
      },
    };
    let id = match known {
      Some(id) => id,
      None => {
        if linked_to_hub {
          continue;
        }
        match import_into_hub(env, &name, &dir, &source_hint(&name, &dir_name)) {
          Ok(id) => {
            report.imported.push(name.clone());
            id
          },
          Err(err) => {
            report
              .errors
              .push(format!("{}: {}", dir.to_string_lossy(), err));
            continue;
          },
        }
      },
    };
    let hub_dir = env.hub_dir(&id, &name);

    // Relink symlinks that used to point at the legacy canonical dir.
    if linked_to_legacy {
      if let Err(err) = write_target(&hub_dir, &dir, InstallMode::Symlink) {
        report
          .errors
          .push(format!("{}: relink failed: {}", dir.to_string_lossy(), err));
        continue;
      }
    } else if is_symlink(&dir) && !linked_to_hub {
      // Linked somewhere we do not manage: leave it alone.
      continue;
    }

    let ids: Vec<String> = if agent_ids.is_empty() {
      vec![crate::services::agent_apps_service::SHARED_AGENTS_APP_ID.to_string()]
    } else {
      agent_ids.clone()
    };
    match register_existing_install(env, &id, &dir, scope, project_path.clone(), &ids) {
      Ok(()) => report.installs += 1,
      Err(err) => report
        .errors
        .push(format!("{}: {}", dir.to_string_lossy(), err)),
    }
  }
}

/// Copy a skill into the hub as a new record and return its id.
fn import_into_hub(
  env: &Env,
  name: &str,
  dir: &Path,
  hint: &Option<(SkillSource, Option<String>)>,
) -> Result<String, String> {
  let id = new_skill_id();
  let hub_dir = env.hub_dir(&id, name);
  copy_dir(dir, &hub_dir, &CopyOpts::skill())?;
  invalidate_hash_cache(&hub_dir);
  let hash = hash_dir(&hub_dir)?;
  let now = now_rfc3339();
  let (source, installed_at) = hint.clone().unwrap_or((SkillSource::None, None));
  let imported_at = installed_at
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| now.clone());
  store(env).update(|lock| {
    lock.skills.insert(
      id.clone(),
      SkillRecord {
        name: name.to_string(),
        source,
        hash,
        imported_at,
        updated_at: now,
        installs: Vec::new(),
      },
    );
    Ok(())
  })?;
  Ok(id)
}

/// Move every version 1 record (keyed by name, hub copy at `skills/<name>`) to a new id and
/// `skills/<id>/<name>`, and relink its symlink installs. Each record moves on its own, so
/// an interrupted run continues on the next call: a hub copy that was moved before the
/// lock was written is found at `skills/<id>/<name>` and keeps that id.
pub fn migrate_skill_ids(env: &Env) -> Result<(), String> {
  if !needs_id_repair(env)? {
    return Ok(());
  }
  let _ops = ops_guard_unless_held(&env.youskill_root)?;
  let lock = store(env).read()?;
  for (key, record) in &lock.skills {
    if !is_skill_id(key) {
      migrate_named_record(env, key, record)?;
    } else if record.name.is_empty() {
      // Recover only from a unique directory; keep the recorded hash and install history.
      if let Some(name) =
        skill_dir_in(&env.hub_root.join(key)).filter(|name| validate_skill_name(name).is_ok())
      {
        store(env).update(|lock| {
          if let Some(record) = lock.skills.get_mut(key) {
            record.name = name;
          }
          Ok(())
        })?;
      }
    }
  }
  Ok(())
}

fn needs_id_repair(env: &Env) -> Result<bool, String> {
  Ok(
    store(env)
      .read()?
      .skills
      .iter()
      .any(|(key, record)| !is_skill_id(key) || record.name.is_empty()),
  )
}

fn migrate_named_record(env: &Env, key: &str, record: &SkillRecord) -> Result<(), String> {
  let name = if record.name.is_empty() {
    key.to_string()
  } else {
    record.name.clone()
  };
  let old_dir = env.hub_root.join(key);
  let id = if old_dir.is_dir() {
    let id = new_skill_id();
    move_dir(&old_dir, &env.hub_dir(&id, &name))?;
    invalidate_hash_cache(&old_dir);
    id
  } else {
    moved_hub_copy(env, &name)?.unwrap_or_else(new_skill_id)
  };

  let hub_dir = env.hub_dir(&id, &name);
  for install in &record.installs {
    let target = Path::new(&install.path);
    if is_symlink(target) && symlink_points_to(target, &old_dir) {
      if let Err(err) = write_target(&hub_dir, target, InstallMode::Symlink) {
        tracing::warn!("relinking {} failed: {}", install.path, err);
      }
    }
  }

  store(env).update(|lock| {
    if let Some(mut moved) = lock.skills.remove(key) {
      moved.name = name.clone();
      lock.skills.insert(id.clone(), moved);
    }
    Ok(())
  })
}

/// The id of `skills/<id>/<name>` when no record has that id yet.
fn moved_hub_copy(env: &Env, name: &str) -> Result<Option<String>, String> {
  let Ok(entries) = fs::read_dir(&env.hub_root) else {
    return Ok(None);
  };
  let known = store(env).read()?.skills;
  Ok(entries.flatten().find_map(|entry| {
    let id = entry.file_name().to_string_lossy().to_string();
    let found = is_skill_id(&id)
      && !known.contains_key(&id)
      && skill_dir_in(&entry.path()).as_deref() == Some(name);
    found.then_some(id)
  }))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::{AgentApp, TargetState, UserProject};
  use crate::services::hub_service::get_hub_skill;

  fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
  }

  fn app(id: &str, global: &str, project: &str) -> AgentApp {
    AgentApp {
      id: id.to_string(),
      display_name: id.to_string(),
      project_path: Some(project.to_string()),
      global_path: Some(global.to_string()),
      detect_path: None,
      profile_path: Some("AGENTS.md".to_string()),
      global_profile_path: None,
      is_user_custom: false,
    }
  }

  #[cfg(unix)]
  #[test]
  fn migrates_legacy_layout_idempotently() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("proj");
    fs::create_dir_all(&project).unwrap();
    let env = Env::for_test(
      tmp.path(),
      vec![
        app("agents", "~/.agents/skills", ".agents/skills"),
        app("claude-code", "~/.claude/skills", ".claude/skills"),
        app("codex", "~/.codex/skills", ".agents/skills"),
      ],
      vec![UserProject {
        name: "proj".to_string(),
        path: project.to_string_lossy().to_string(),
        workspace_path: None,
      }],
    );
    let legacy = env.home.join(".agents/skills");
    write(&legacy, "foo/SKILL.md", "---\nname: foo\n---\nF");
    write(&legacy, "bar/SKILL.md", "---\nname: bar\n---\nB");
    write(
      &env.home.join(".agents"),
      ".skill-lock.json",
      r#"{"version":3,"skills":{"foo":{"source":"o/r","sourceType":"github","sourceUrl":"https://github.com/o/r.git","skillPath":"skills/foo/SKILL.md","skillFolderHash":"sha1","installedAt":"2026-01-01T00:00:00Z","updatedAt":"x"}}}"#,
    );
    write(
      &env.config_dir,
      "native-skill-lock.json",
      r#"{"version":1,"skills":{"bar":{"installedAt":"2026-02-02T00:00:00Z","updatedAt":"x"}}}"#,
    );
    // claude: symlink to legacy foo, real copy of bar (edited), plus an unmanaged skill.
    let claude = env.home.join(".claude/skills");
    fs::create_dir_all(&claude).unwrap();
    std::os::unix::fs::symlink(legacy.join("foo"), claude.join("foo")).unwrap();
    write(&claude, "bar/SKILL.md", "---\nname: bar\n---\nB-edited");
    write(&claude, "solo/SKILL.md", "---\nname: solo\n---\nS");
    // codex: symlink pointing somewhere else entirely.
    let codex = env.home.join(".codex/skills");
    fs::create_dir_all(&codex).unwrap();
    let elsewhere = tmp.path().join("elsewhere/foo");
    write(&elsewhere, "SKILL.md", "---\nname: foo\n---\nX");
    std::os::unix::fs::symlink(&elsewhere, codex.join("foo")).unwrap();
    // project: .agents/skills/baz with github project lock.
    write(
      &project,
      ".agents/skills/baz/SKILL.md",
      "---\nname: baz\n---\nZ",
    );
    write(
      &project,
      "skills-lock.json",
      r#"{"version":1,"skills":{"baz":{"source":"p/q","sourceType":"github","computedHash":"c"}}}"#,
    );

    let report = migrate_legacy(&env).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let mut imported = report.imported.clone();
    imported.sort();
    assert_eq!(imported, vec!["bar", "baz", "foo", "solo"]);

    let foo = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(foo.source.github_repo(), Some("o/r"));
    assert_eq!(foo.imported_at, "2026-01-01T00:00:00Z");
    assert_eq!(foo.hub_state, crate::models::HubState::Ok);
    // legacy dir recorded for `agents`, claude relinked to the hub.
    let by_path = |v: &crate::models::HubSkillView, suffix: &str| {
      v.installs
        .iter()
        .find(|i| i.record.path.ends_with(suffix))
        .cloned()
        .unwrap_or_else(|| panic!("no install ending with {suffix}"))
    };
    assert_eq!(
      by_path(&foo, ".agents/skills/foo").record.agent_ids,
      vec!["agents"]
    );
    let claude_foo = by_path(&foo, ".claude/skills/foo");
    assert_eq!(claude_foo.record.mode, InstallMode::Symlink);
    assert_eq!(claude_foo.state, TargetState::InSync);
    assert!(symlink_points_to(
      &claude.join("foo"),
      &env.hub_dir_of("foo")
    ));
    // codex link to an unmanaged location is left alone and not recorded.
    assert!(foo
      .installs
      .iter()
      .all(|i| !i.record.path.contains(".codex")));
    assert!(symlink_points_to(&codex.join("foo"), &elsewhere));

    let bar = get_hub_skill(&env, "bar").unwrap().unwrap();
    assert_eq!(bar.source, SkillSource::None);
    assert_eq!(bar.imported_at, "2026-02-02T00:00:00Z");
    let claude_bar = by_path(&bar, ".claude/skills/bar");
    assert_eq!(claude_bar.record.mode, InstallMode::Copy);
    assert_eq!(claude_bar.state, TargetState::Modified);

    let solo = get_hub_skill(&env, "solo").unwrap().unwrap();
    assert_eq!(solo.installs.len(), 1);
    assert_eq!(solo.installs[0].state, TargetState::InSync);

    let baz = get_hub_skill(&env, "baz").unwrap().unwrap();
    assert_eq!(baz.source.github_repo(), Some("p/q"));
    let baz_install = &baz.installs[0];
    assert_eq!(baz_install.record.scope, InstallScope::Project);
    let mut ids = baz_install.record.agent_ids.clone();
    ids.sort();
    assert_eq!(ids, vec!["agents", "codex"]);

    // Legacy files untouched.
    assert!(legacy.join("foo/SKILL.md").is_file());
    assert!(env.home.join(".agents/.skill-lock.json").is_file());

    // Re-run: skill records are unchanged (only the migration report is rewritten).
    let before = crate::services::lock_service::read_lock_file(&env.lock_path).unwrap();
    let second = migrate_legacy(&env).unwrap();
    assert!(second.imported.is_empty());
    assert!(second.errors.is_empty());
    let after = crate::services::lock_service::read_lock_file(&env.lock_path).unwrap();
    assert_eq!(before.skills, after.skills);
    assert!(!needs_migration(&env).unwrap());
  }

  #[test]
  fn legacy_app_specific_user_roots_become_installs_of_that_app() {
    let tmp = tempfile::tempdir().unwrap();
    let env = Env::for_test(
      tmp.path(),
      vec![
        app("agents", "~/.agents/skills", ".agents/skills"),
        app("cursor", "~/.agents/skills", ".agents/skills"),
      ],
      vec![],
    );
    // Installed by the previous app into Cursor's own directory.
    write(
      &env.home.join(".cursor/skills"),
      "foo/SKILL.md",
      "---\nname: foo\n---\nF",
    );

    let report = migrate_legacy(&env).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.imported, vec!["foo"]);

    let foo = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(foo.installs.len(), 1);
    assert!(foo.installs[0].record.path.ends_with(".cursor/skills/foo"));
    assert_eq!(foo.installs[0].record.agent_ids, vec!["cursor"]);
    assert_eq!(foo.installs[0].state, TargetState::InSync);
  }

  /// A version 1 lock: `foo` keyed by name at `skills/foo`, installed as a symlink into
  /// Claude Code and as a copy into the shared directory.
  #[cfg(unix)]
  fn v1_hub(tmp: &Path) -> Env {
    use crate::models::InstallRecord;
    let env = Env::for_test(
      tmp,
      vec![
        app("agents", "~/.agents/skills", ".agents/skills"),
        app("claude-code", "~/.claude/skills", ".claude/skills"),
      ],
      vec![],
    );
    let old_dir = env.hub_root.join("foo");
    write(&old_dir, "SKILL.md", "---\nname: foo\n---\nF");
    let hash = hash_dir(&old_dir).unwrap();
    let link = env.home.join(".claude/skills/foo");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&old_dir, &link).unwrap();
    let copy = env.home.join(".agents/skills/foo");
    copy_dir(&old_dir, &copy, &CopyOpts::skill()).unwrap();
    let install = |path: &Path, mode: InstallMode, agent: &str| InstallRecord {
      scope: InstallScope::User,
      project_path: None,
      path: path.to_string_lossy().to_string(),
      mode,
      hash: hash.clone(),
      installed_at: "t".to_string(),
      agent_ids: vec![agent.to_string()],
    };
    let installs = vec![
      install(&link, InstallMode::Symlink, "claude-code"),
      install(&copy, InstallMode::Copy, "agents"),
    ];
    write(
      &env.youskill_root,
      ".skill-lock.json",
      &serde_json::json!({
        "version": 1,
        "skills": { "foo": {
          "hash": hash,
          "importedAt": "t",
          "updatedAt": "t",
          "installs": installs,
        } }
      })
      .to_string(),
    );
    env
  }

  #[cfg(unix)]
  #[test]
  fn version_1_records_move_to_ids_and_symlinks_follow() {
    let tmp = tempfile::tempdir().unwrap();
    let env = v1_hub(tmp.path());

    migrate_skill_ids(&env).unwrap();

    let lock = store(&env).read().unwrap();
    assert_eq!(lock.skills.len(), 1);
    let (id, record) = lock.skills.iter().next().unwrap();
    assert!(is_skill_id(id));
    assert_eq!(record.name, "foo");
    let hub_dir = env.hub_dir(id, "foo");
    assert!(hub_dir.join("SKILL.md").is_file());
    assert!(!env.hub_root.join("foo").exists());
    assert!(symlink_points_to(
      &env.home.join(".claude/skills/foo"),
      &hub_dir
    ));

    let view = get_hub_skill(&env, "foo").unwrap().unwrap();
    assert_eq!(&view.id, id);
    assert_eq!(view.hub_state, crate::models::HubState::Ok);
    assert!(view
      .installs
      .iter()
      .all(|install| install.state == TargetState::InSync));

    // A second run changes nothing.
    migrate_skill_ids(&env).unwrap();
    assert_eq!(store(&env).read().unwrap().skills, lock.skills);
  }

  #[cfg(unix)]
  #[test]
  fn interrupted_id_migration_keeps_the_moved_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let env = v1_hub(tmp.path());
    // Stopped after the move: the hub copy has an id, the lock and the link do not.
    let id = new_skill_id();
    move_dir(&env.hub_root.join("foo"), &env.hub_dir(&id, "foo")).unwrap();

    migrate_skill_ids(&env).unwrap();

    let lock = store(&env).read().unwrap();
    assert_eq!(lock.skills.keys().collect::<Vec<_>>(), vec![&id]);
    assert!(symlink_points_to(
      &env.home.join(".claude/skills/foo"),
      &env.hub_dir(&id, "foo")
    ));
    let view = get_hub_skill(&env, &id).unwrap().unwrap();
    assert_eq!(view.installs.len(), 2);
    assert!(view
      .installs
      .iter()
      .all(|install| install.state == TargetState::InSync));
  }

  #[cfg(unix)]
  #[test]
  fn id_records_with_empty_names_recover_without_changing_identity_or_installs() {
    let tmp = tempfile::tempdir().unwrap();
    let env = v1_hub(tmp.path());
    migrate_skill_ids(&env).unwrap();
    let expected = store(&env).read().unwrap();
    let id = expected.skills.keys().next().unwrap();
    store(&env)
      .update(|lock| {
        lock.skills.get_mut(id).unwrap().name.clear();
        Ok(())
      })
      .unwrap();

    let view = get_hub_skill(&env, id).unwrap().unwrap();
    assert_eq!(view.name, "foo");
    assert_eq!(view.hub_state, crate::models::HubState::Ok);
    assert!(view
      .installs
      .iter()
      .all(|install| install.state == TargetState::InSync));
    assert_eq!(store(&env).read().unwrap(), expected);
    migrate_skill_ids(&env).unwrap();
    assert_eq!(store(&env).read().unwrap(), expected);
  }

  #[test]
  fn empty_names_are_not_guessed_when_the_hub_is_missing_or_ambiguous() {
    let tmp = tempfile::tempdir().unwrap();
    let env = Env::for_test(tmp.path(), vec![], vec![]);
    let id = new_skill_id();
    write(
      &env.youskill_root,
      ".skill-lock.json",
      &serde_json::json!({
        "version": 2,
        "skills": { id.clone(): {
          "name": "", "hash": "h", "importedAt": "t", "updatedAt": "t"
        }}
      })
      .to_string(),
    );
    let before = fs::read(&env.lock_path).unwrap();
    migrate_skill_ids(&env).unwrap();
    assert_eq!(fs::read(&env.lock_path).unwrap(), before);
    write(
      &env.hub_dir(&id, "foo"),
      "SKILL.md",
      "---\nname: foo\n---\nF",
    );
    write(
      &env.hub_dir(&id, "bar"),
      "SKILL.md",
      "---\nname: bar\n---\nB",
    );
    migrate_skill_ids(&env).unwrap();
    assert_eq!(fs::read(&env.lock_path).unwrap(), before);
  }

  #[test]
  fn fresh_machine_migration_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let env = Env::for_test(tmp.path(), vec![], vec![]);
    assert!(needs_migration(&env).unwrap());
    let report = migrate_legacy(&env).unwrap();
    assert!(report.imported.is_empty());
    assert!(env.lock_path.is_file());
    assert!(!needs_migration(&env).unwrap());
  }
}
