use super::*;
use crate::models::InstructionRecord;
use crate::utils::git_remote::{RemoteCommit, RemoteHead};
use std::collections::HashMap;
use std::sync::Mutex;

/// An in-memory repository with GitHub's fast-forward-only branch update.
#[derive(Default)]
struct FakeRepo {
  inner: Mutex<FakeInner>,
}

#[derive(Default)]
struct FakeInner {
  blobs: HashMap<String, Vec<u8>>,
  trees: HashMap<String, BTreeMap<String, TreeFile>>,
  /// Commit -> (tree, parent, message).
  commits: HashMap<String, (String, Option<String>, String)>,
  head: Option<String>,
  counter: u64,
}

impl FakeRepo {
  /// A new repository with only a README, as GitHub creates it.
  fn initialized() -> Self {
    let repo = FakeRepo::default();
    let readme = b"# lib\n".to_vec();
    let sha = repo.create_blob(&readme).unwrap();
    let tree = repo
      .create_tree(&BTreeMap::from([(
        "README.md".to_string(),
        TreeFile {
          mode: MODE_FILE.to_string(),
          sha,
        },
      )]))
      .unwrap();
    let commit = repo.create_commit("init", &tree, None).unwrap();
    repo.update_head(&commit).unwrap();
    repo
  }

  fn head_file(&self, path: &str) -> Option<Vec<u8>> {
    let inner = self.inner.lock().unwrap();
    let commit = inner.head.as_ref()?;
    let tree = &inner.trees[&inner.commits[commit].0];
    tree.get(path).map(|file| inner.blobs[&file.sha].clone())
  }

  fn commit_count(&self) -> usize {
    self.inner.lock().unwrap().commits.len()
  }
}

impl RemoteRepo for FakeRepo {
  fn head(&self) -> Result<Option<RemoteHead>, String> {
    let inner = self.inner.lock().unwrap();
    Ok(inner.head.as_ref().map(|commit| RemoteHead {
      commit: commit.clone(),
      tree: inner.commits[commit].0.clone(),
    }))
  }

  fn tree(&self, tree: &str) -> Result<BTreeMap<String, TreeFile>, String> {
    Ok(self.inner.lock().unwrap().trees[tree].clone())
  }

  fn download(
    &self,
    _commit: &str,
    files: &BTreeMap<String, TreeFile>,
  ) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let inner = self.inner.lock().unwrap();
    Ok(
      files
        .iter()
        .map(|(path, file)| (path.clone(), inner.blobs[&file.sha].clone()))
        .collect(),
    )
  }

  fn create_blob(&self, content: &[u8]) -> Result<String, String> {
    let sha = blob_sha(content);
    self
      .inner
      .lock()
      .unwrap()
      .blobs
      .insert(sha.clone(), content.to_vec());
    Ok(sha)
  }

  fn create_tree(&self, files: &BTreeMap<String, TreeFile>) -> Result<String, String> {
    let mut inner = self.inner.lock().unwrap();
    for (path, file) in files {
      if !inner.blobs.contains_key(&file.sha) {
        return Err(format!("missing blob for {}", path));
      }
    }
    inner.counter += 1;
    let id = format!("tree{}", inner.counter);
    inner.trees.insert(id.clone(), files.clone());
    Ok(id)
  }

  fn create_commit(
    &self,
    message: &str,
    tree: &str,
    parent: Option<&str>,
  ) -> Result<String, String> {
    let mut inner = self.inner.lock().unwrap();
    inner.counter += 1;
    let id = format!("commit{}", inner.counter);
    inner
      .commits
      .insert(
        id.clone(),
        (tree.to_string(), parent.map(String::from), message.to_string()),
      );
    Ok(id)
  }

  fn update_head(&self, commit: &str) -> Result<(), UpdateError> {
    let mut inner = self.inner.lock().unwrap();
    let parent = inner.commits[commit].1.clone();
    if parent != inner.head {
      return Err(UpdateError::Rejected);
    }
    inner.head = Some(commit.to_string());
    Ok(())
  }

  fn history(&self, limit: usize) -> Result<Vec<RemoteCommit>, String> {
    let inner = self.inner.lock().unwrap();
    let mut commits = Vec::new();
    let mut next = inner.head.clone();
    while let Some(id) = next {
      if commits.len() == limit {
        break;
      }
      let (tree, parent, message) = &inner.commits[&id];
      commits.push(RemoteCommit {
        id: id.clone(),
        tree: tree.clone(),
        time: format!("2026-10-01T00:00:{:02}Z", commits.len()),
        message: message.clone(),
      });
      next = parent.clone();
    }
    Ok(commits)
  }
}

const FOO: &str = "00000000-0000-4000-8000-000000000001";
const BAR: &str = "00000000-0000-4000-8000-000000000002";
const NOTE: &str = "00000000-0000-4000-8000-0000000000aa";

fn app(id: &str) -> AgentApp {
  AgentApp {
    id: id.to_string(),
    display_name: id.to_string(),
    project_path: Some(format!(".{}/skills", id)),
    global_path: Some(format!("~/.{}/skills", id)),
    detect_path: None,
    profile_path: Some("AGENTS.md".to_string()),
    global_profile_path: Some(format!("~/.{}/AGENTS.md", id)),
    is_user_custom: false,
  }
}

fn computer(root: &Path, name: &str, agents: &[&str]) -> Env {
  let env = Env::for_test(
    &root.join(name),
    agents.iter().map(|id| app(id)).collect(),
    Vec::new(),
  );
  set_connection(&env, "me/youskill-library", "main", name).unwrap();
  env
}

fn add_skill(env: &Env, id: &str, name: &str, body: &str) {
  let dir = env.hub_dir(id, name);
  fs::create_dir_all(dir.join("scripts")).unwrap();
  fs::write(
    dir.join("SKILL.md"),
    format!("---\nname: {}\n---\n{}", name, body),
  )
  .unwrap();
  fs::write(dir.join("scripts/run.sh"), "echo hi\n").unwrap();
  let hash = hash_dir(&dir).unwrap();
  store(env)
    .update(|lock| {
      lock.skills.insert(
        id.to_string(),
        SkillRecord {
          name: name.to_string(),
          source: SkillSource::None,
          hash,
          imported_at: "2026-10-01T00:00:00Z".to_string(),
          updated_at: "2026-10-01T00:00:00Z".to_string(),
          installs: Vec::new(),
        },
      );
      Ok(())
    })
    .unwrap();
}

fn edit_skill(env: &Env, id: &str, name: &str, body: &str) {
  let dir = env.hub_dir(id, name);
  fs::write(
    dir.join("SKILL.md"),
    format!("---\nname: {}\n---\n{}", name, body),
  )
  .unwrap();
  invalidate_hash_cache(&dir);
  hub_service::accept_hub(env, id).unwrap();
}

fn add_instruction(env: &Env, id: &str, body: &str) {
  fs::create_dir_all(&env.instructions_root).unwrap();
  fs::write(env.instruction_file(id), body).unwrap();
  let hash = crate::utils::hash::hash_file(&env.instruction_file(id)).unwrap();
  instruction_service::update_lock(env, |lock| {
    lock.instructions.insert(
      id.to_string(),
      InstructionRecord {
        name: "Shared rules".to_string(),
        source: InstructionSource::None,
        hash,
        imported_at: "2026-10-01T00:00:00Z".to_string(),
        updated_at: "2026-10-01T00:00:00Z".to_string(),
        installs: Vec::new(),
      },
    );
    Ok(())
  })
  .unwrap();
}

fn install(env: &Env, id: &str, agents: &[&str]) {
  let result = install_service::install_skill(
    env,
    InstallRequest {
      name: id.to_string(),
      targets: agents
        .iter()
        .map(|agent| InstallTargetSpec {
          scope: InstallScope::User,
          project_path: None,
          agent_id: agent.to_string(),
        })
        .collect(),
      mode: Some(InstallMode::Copy),
      force: false,
    },
  )
  .unwrap();
  assert!(result.applied, "{:?}", result.blockers);
}

fn hub_hash(env: &Env, id: &str, name: &str) -> String {
  hash_dir(&env.hub_dir(id, name)).unwrap()
}

fn library_at_head(repo: &FakeRepo) -> LibraryFile {
  serde_json::from_slice(&repo.head_file(LIBRARY_FILE).unwrap()).unwrap()
}

#[test]
fn backup_then_restore_on_an_empty_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code", "codex"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  add_instruction(&a, NOTE, "# rules\n");
  fs::write(
    a.config_dir.join(USER_AGENT_APPS_FILE),
    serde_json::to_string(&vec![AgentApp {
      is_user_custom: true,
      ..app("my-agent")
    }])
    .unwrap(),
  )
  .unwrap();

  let backup = backup_with(&a, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  assert_eq!(backup.changes.len(), 3, "{:?}", backup.changes);
  assert!(status(&a).unwrap().changes.is_empty());

  let b = computer(tmp.path(), "b", &["claude-code", "codex"]);
  assert!(status(&b).unwrap().library_empty);
  let pulled = pull_with(&b, &repo).unwrap();
  assert_eq!(pulled.result, PullResult::Pulled);
  assert!(pulled.errors.is_empty(), "{:?}", pulled.errors);
  assert!(pulled.agents_changed);
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert_eq!(
    fs::read(b.instruction_file(NOTE)).unwrap(),
    b"# rules\n".to_vec()
  );
  assert!(status(&b).unwrap().changes.is_empty());

  let items = pending(&b).unwrap();
  assert_eq!(
    items
      .iter()
      .map(|item| (item.action, item.id.as_str(), item.agent_id.as_str()))
      .collect::<Vec<_>>(),
    vec![(PendingAction::Install, FOO, "claude-code")]
  );
  let applied = apply_pending(&b, &items, InstallMode::Copy).unwrap();
  assert_eq!(applied.applied, 1, "{:?}", applied.failures);
  assert!(b.home.join(".claude-code/skills/foo/SKILL.md").is_file());
  assert!(pending(&b).unwrap().is_empty());
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::UpToDate
  );
}

#[test]
fn repository_holds_no_paths_of_this_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();

  let library = String::from_utf8(repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  assert!(!library.contains(&a.home.to_string_lossy().to_string()));
  assert_eq!(
    library_at_head(&repo).skills[FOO].agents,
    vec!["claude-code"]
  );
  assert_eq!(
    repo.head_file(&format!("skills/{}/foo/scripts/run.sh", FOO)),
    Some(b"echo hi\n".to_vec())
  );
}

#[test]
fn unchanged_library_makes_no_commit() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  let commits = repo.commit_count();
  assert_eq!(
    backup_with(&a, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  assert_eq!(repo.commit_count(), commits);
}

#[test]
fn pull_brings_changes_from_another_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  add_skill(&a, BAR, "bar", "v1");
  backup_with(&a, &repo).unwrap();
  let b = computer(tmp.path(), "b", &["claude-code"]);
  pull_with(&b, &repo).unwrap();

  edit_skill(&a, FOO, "foo", "v2");
  hub_service::remove_hub_skill(&a, BAR, false).unwrap();
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);

  let check = check_remote_with(&b, &repo).unwrap();
  assert!(check.remote_ahead);
  assert_eq!(check.changes.len(), 2);
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::RemoteAhead
  );

  let pulled = pull_with(&b, &repo).unwrap();
  assert_eq!(pulled.result, PullResult::Pulled);
  assert!(pulled.errors.is_empty(), "{:?}", pulled.errors);
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert!(!store(&b).read().unwrap().skills.contains_key(BAR));
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(pull_with(&b, &repo).unwrap().result, PullResult::UpToDate);
}

#[test]
fn both_sides_changed_writes_nothing() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  let b = computer(tmp.path(), "b", &["claude-code"]);
  pull_with(&b, &repo).unwrap();

  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, FOO, "foo", "from b");
  let before = hub_hash(&b, FOO, "foo");

  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::Diverged
  );
  assert_eq!(pull_with(&b, &repo).unwrap().result, PullResult::Diverged);
  assert_eq!(hub_hash(&b, FOO, "foo"), before);
}

#[test]
fn agents_missing_on_this_computer_stay_enabled() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code", "cursor"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code", "cursor"]);
  backup_with(&a, &repo).unwrap();

  // B has no Cursor. It installs for Claude Code and backs up another change.
  let b = computer(tmp.path(), "b", &["claude-code"]);
  pull_with(&b, &repo).unwrap();
  let items = pending(&b).unwrap();
  assert_eq!(items.len(), 1);
  apply_pending(&b, &items, InstallMode::Copy).unwrap();
  add_skill(&b, BAR, "bar", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert_eq!(
    library_at_head(&repo).skills[FOO].agents,
    vec!["claude-code", "cursor"]
  );

  // Uninstalling here disables the agent in the library.
  install_service::uninstall_skill(
    &b,
    UninstallRequest {
      name: FOO.to_string(),
      targets: vec![InstallTargetSpec {
        scope: InstallScope::User,
        project_path: None,
        agent_id: "claude-code".to_string(),
      }],
      paths: Vec::new(),
      force: false,
    },
  )
  .unwrap();
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert_eq!(library_at_head(&repo).skills[FOO].agents, vec!["cursor"]);

  // A pulls: Claude Code is no longer enabled there.
  pull_with(&a, &repo).unwrap();
  let items = pending(&a).unwrap();
  assert_eq!(
    items
      .iter()
      .map(|item| (item.action, item.agent_id.as_str()))
      .collect::<Vec<_>>(),
    vec![(PendingAction::Uninstall, "claude-code")]
  );
}

#[test]
fn first_sync_against_an_existing_library_is_refused() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "bar", "v1");
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::RemoteNotEmpty
  );
  assert_eq!(
    pull_with(&b, &repo).unwrap().result,
    PullResult::LocalNotEmpty
  );
  assert!(!store(&b).read().unwrap().skills.contains_key(FOO));
}

#[test]
fn effective_agents_apply_only_local_changes() {
  let base = vec!["a".to_string(), "b".to_string(), "c".to_string()];
  let base_local = vec!["a".to_string(), "b".to_string()];
  let now_local = vec!["b".to_string(), "d".to_string()];
  assert_eq!(
    effective_agents(Some(&base), Some(&base_local), &now_local),
    vec!["b", "c", "d"]
  );
  assert_eq!(effective_agents(None, None, &now_local), vec!["b", "d"]);
}

#[test]
fn unsafe_repository_paths_are_rejected() {
  assert!(safe_components("skills/x/foo/SKILL.md").is_some());
  assert!(safe_components("skills/x/../../etc").is_none());
  assert!(safe_components("/abs").is_none());
  assert!(safe_components("skills/x/foo/a\\b").is_none());
}
