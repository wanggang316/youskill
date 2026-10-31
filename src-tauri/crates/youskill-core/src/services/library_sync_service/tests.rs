use super::restore::safe_components;
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
  fail_downloads: bool,
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

  fn head_commit(&self) -> String {
    self.inner.lock().unwrap().head.clone().unwrap()
  }

  fn parent_of(&self, commit: &str) -> Option<String> {
    self.inner.lock().unwrap().commits[commit].1.clone()
  }

  /// Files of a commit under `prefix`.
  fn files_at(&self, commit: &str, prefix: &str) -> BTreeMap<String, TreeFile> {
    let inner = self.inner.lock().unwrap();
    inner.trees[&inner.commits[commit].0]
      .iter()
      .filter(|(path, _)| path.starts_with(prefix))
      .map(|(path, file)| (path.clone(), file.clone()))
      .collect()
  }

  fn fail_downloads(&self) {
    self.inner.lock().unwrap().fail_downloads = true;
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
    // The library file still loads, so a restore fails after it started writing.
    if inner.fail_downloads && files.keys().any(|path| path != LIBRARY_FILE) {
      return Err("network down".to_string());
    }
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
    inner.commits.insert(
      id.clone(),
      (
        tree.to_string(),
        parent.map(String::from),
        message.to_string(),
      ),
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
  install_as(env, id, agents, InstallMode::Copy);
}

fn install_as(env: &Env, id: &str, agents: &[&str], mode: InstallMode) {
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
      mode: Some(mode),
      force: false,
    },
  )
  .unwrap();
  assert!(result.applied, "{:?}", result.blockers);
}

fn plan(env: &Env, repo: &FakeRepo, mode: RestoreMode) -> RestorePlan {
  plan_restore_with(env, repo, mode, None).unwrap()
}

fn apply(
  env: &Env,
  repo: &FakeRepo,
  plan: &RestorePlan,
  decisions: RestoreDecisions,
) -> RestoreOutcome {
  apply_restore_with(
    env,
    repo,
    plan,
    &decisions,
    &RestoreOptions::default(),
    InstallMode::Copy,
  )
  .unwrap()
}

/// Plan and apply a normal restore with the default decisions.
fn pull(env: &Env, repo: &FakeRepo) -> RestoreOutcome {
  let plan = plan(env, repo, RestoreMode::Pull);
  apply(env, repo, &plan, RestoreDecisions::default())
}

fn item<'a>(plan: &'a RestorePlan, key: &str) -> &'a RestoreItem {
  plan
    .items
    .iter()
    .find(|item| item.key == key)
    .unwrap_or_else(|| panic!("{} not in {:?}", key, plan.items))
}

fn resolutions(plan: &RestorePlan) -> Vec<(String, Resolution)> {
  plan
    .items
    .iter()
    .map(|item| (item.key.clone(), item.resolution))
    .collect()
}

fn decide(conflicts: &[(&str, ConflictChoice)]) -> RestoreDecisions {
  RestoreDecisions {
    conflicts: conflicts
      .iter()
      .map(|(key, choice)| (key.to_string(), *choice))
      .collect(),
    ..Default::default()
  }
}

fn set_source(env: &Env, id: &str, source: SkillSource) {
  store(env)
    .update(|lock| {
      lock.skills.get_mut(id).unwrap().source = source;
      Ok(())
    })
    .unwrap();
}

fn github_source(dir: &str) -> SkillSource {
  SkillSource::Github {
    repo: "acme/skills".to_string(),
    url: "https://github.com/acme/skills".to_string(),
    skill_path: format!("{}/SKILL.md", dir),
    branch: Some("main".to_string()),
    remote_sha: None,
    marketplace_id: None,
    latest_remote_sha: None,
    checked_at: None,
  }
}

fn skill_body(env: &Env, id: &str, name: &str) -> String {
  fs::read_to_string(env.hub_dir(id, name).join("SKILL.md")).unwrap()
}

/// Two computers on one repository: A backs up `setup`, B restores it.
fn synced_pair(root: &Path, repo: &FakeRepo, setup: impl FnOnce(&Env)) -> (Env, Env) {
  let a = computer(root, "a", &["claude-code"]);
  setup(&a);
  assert_eq!(backup_with(&a, repo).unwrap().result, BackupResult::Pushed);
  let b = computer(root, "b", &["claude-code"]);
  assert_eq!(pull(&b, repo).result, RestoreResult::Applied);
  (a, b)
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
  let pulled = pull(&b, &repo);
  assert_eq!(pulled.result, RestoreResult::Applied);
  assert!(!pulled.pushed);
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
fn pending_install_over_other_content_reports_the_item() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  let target = b.home.join(".claude-code/skills/foo");
  fs::create_dir_all(&target).unwrap();
  fs::write(target.join("SKILL.md"), "other").unwrap();
  pull(&b, &repo);

  let items = pending(&b).unwrap();
  let applied = apply_pending(&b, &items, InstallMode::Copy).unwrap();
  assert_eq!(applied.applied, 0);
  assert_eq!(applied.failures.len(), 1, "{:?}", applied.failures);
  assert_eq!(applied.failures[0].kind, ItemKind::Skill);
  assert_eq!(applied.failures[0].name, "foo");
  assert!(applied.failures[0].reason.contains("foo"));
  assert_eq!(
    fs::read_to_string(target.join("SKILL.md")).unwrap(),
    "other"
  );
  assert_eq!(pending(&b).unwrap(), items);
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
  pull(&b, &repo);

  edit_skill(&a, FOO, "foo", "v2");
  hub_service::remove_hub_skill(&a, BAR, false).unwrap();
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::RemoteAhead
  );

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(planned.state, PlanState::Ready);
  assert_eq!(
    resolutions(&planned),
    vec![
      (skill_key(BAR), Resolution::RemovalCandidate),
      (skill_key(FOO), Resolution::TakeRemote),
    ]
  );
  let pulled = apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      removals: BTreeMap::from([(skill_key(BAR), RemovalChoice::Remove)]),
      ..Default::default()
    },
  );
  assert_eq!(pulled.result, RestoreResult::Applied);
  assert!(!pulled.pushed);
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert!(!store(&b).read().unwrap().skills.contains_key(BAR));
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    plan(&b, &repo, RestoreMode::Pull).state,
    PlanState::UpToDate
  );
}

#[test]
fn a_conflict_needs_a_decision() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));

  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, FOO, "foo", "from b");
  let before = hub_hash(&b, FOO, "foo");
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::Diverged
  );

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![(skill_key(FOO), Resolution::Conflict)]
  );
  let refused = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions::default(),
    InstallMode::Copy,
  );
  assert!(refused.is_err());
  assert_eq!(hub_hash(&b, FOO, "foo"), before);

  // Keep local: this computer's version goes to the repository.
  let kept = apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(FOO), ConflictChoice::KeepLocal)]),
  );
  assert!(kept.pushed);
  assert_eq!(hub_hash(&b, FOO, "foo"), before);
  assert!(String::from_utf8(
    repo
      .head_file(&format!("skills/{}/foo/SKILL.md", FOO))
      .unwrap()
  )
  .unwrap()
  .contains("from b"));
  assert!(status(&b).unwrap().changes.is_empty());

  // A takes the repository version of its own conflict.
  edit_skill(&a, FOO, "foo", "from a again");
  let planned = plan(&a, &repo, RestoreMode::Pull);
  assert_eq!(
    item(&planned, &skill_key(FOO)).resolution,
    Resolution::Conflict
  );
  let taken = apply(
    &a,
    &repo,
    &planned,
    decide(&[(&skill_key(FOO), ConflictChoice::TakeRemote)]),
  );
  assert!(!taken.pushed);
  assert_eq!(hub_hash(&a, FOO, "foo"), before);
  assert!(status(&a).unwrap().changes.is_empty());
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
  pull(&b, &repo);
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
  let pulled = pull(&a, &repo);
  assert!(!pulled.pushed);
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
fn first_sync_against_an_existing_library_asks_for_a_mode() {
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
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(planned.state, PlanState::ChooseMode);
  assert!(apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions::default(),
    InstallMode::Copy,
  )
  .is_err());
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

const BAZ: &str = "00000000-0000-4000-8000-000000000003";

#[test]
fn changes_on_different_items_merge_without_asking() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
  });
  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, BAR, "bar", "from b");
  add_skill(&b, BAZ, "baz", "new on b");

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![
      (skill_key(BAR), Resolution::KeepLocal),
      (skill_key(BAZ), Resolution::KeepLocal),
      (skill_key(FOO), Resolution::TakeRemote),
    ]
  );
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert_eq!(outcome.result, RestoreResult::Applied);
  assert!(outcome.pushed);
  assert_eq!(repo.parent_of(&repo.head_commit()), status_commit(&a));
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert!(status(&b).unwrap().changes.is_empty());

  // A fast-forwards to the merge.
  let pulled = pull(&a, &repo);
  assert!(!pulled.pushed);
  assert_eq!(hub_hash(&a, BAR, "bar"), hub_hash(&b, BAR, "bar"));
  assert_eq!(hub_hash(&a, BAZ, "baz"), hub_hash(&b, BAZ, "baz"));
  assert!(status(&a).unwrap().changes.is_empty());
}

fn status_commit(env: &Env) -> Option<String> {
  read_state(env).unwrap().last_synced_commit
}

#[test]
fn conflict_matrix_for_skills() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
    add_skill(a, BAZ, "baz", "v1");
  });
  // FOO: both changed to the same files. BAR: removed here, modified there. BAZ: modified
  // here, removed there.
  edit_skill(&a, FOO, "foo", "same");
  edit_skill(&a, BAR, "bar", "from a");
  hub_service::remove_hub_skill(&a, BAZ, false).unwrap();
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, FOO, "foo", "same");
  hub_service::remove_hub_skill(&b, BAR, false).unwrap();
  edit_skill(&b, BAZ, "baz", "from b");

  let planned = plan(&b, &repo, RestoreMode::Pull);
  let bar = item(&planned, &skill_key(BAR));
  assert_eq!(
    (bar.local_change, bar.remote_change, bar.resolution),
    (
      ItemChange::Removed,
      ItemChange::Modified,
      Resolution::Conflict
    )
  );
  let baz = item(&planned, &skill_key(BAZ));
  assert_eq!(
    (baz.local_change, baz.remote_change, baz.resolution),
    (
      ItemChange::Modified,
      ItemChange::Removed,
      Resolution::Conflict
    )
  );
  assert!(planned
    .items
    .iter()
    .all(|item| item.key != skill_key(FOO) || item.resolution != Resolution::Conflict));

  // Take the repository side of both: BAR comes back, BAZ goes.
  apply(
    &b,
    &repo,
    &planned,
    decide(&[
      (&skill_key(BAR), ConflictChoice::TakeRemote),
      (&skill_key(BAZ), ConflictChoice::TakeRemote),
    ]),
  );
  assert_eq!(hub_hash(&a, BAR, "bar"), hub_hash(&b, BAR, "bar"));
  assert!(!store(&b).read().unwrap().skills.contains_key(BAZ));
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn removal_candidates_are_kept_unless_confirmed() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_instruction(a, NOTE, "# rules\n");
  });
  hub_service::remove_hub_skill(&a, FOO, false).unwrap();
  instruction_service::remove_instruction(&a, NOTE, false).unwrap();
  backup_with(&a, &repo).unwrap();

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![
      (skill_key(FOO), Resolution::RemovalCandidate),
      (instruction_key(NOTE), Resolution::RemovalCandidate),
    ]
  );
  let outcome = apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      removals: BTreeMap::from([(instruction_key(NOTE), RemovalChoice::Remove)]),
      ..Default::default()
    },
  );
  assert!(outcome.pushed, "the kept skill goes back to the repository");
  assert!(store(&b).read().unwrap().skills.contains_key(FOO));
  assert!(!b.instruction_file(NOTE).exists());
  assert!(library_at_head(&repo).skills.contains_key(FOO));
  assert!(!library_at_head(&repo).instructions.contains_key(NOTE));
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn instructions_and_custom_agents_conflict_per_item() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_instruction(a, NOTE, "# rules\n"));
  let write_agents = |env: &Env, id: &str| {
    fs::write(
      env.config_dir.join(USER_AGENT_APPS_FILE),
      serde_json::to_string(&vec![AgentApp {
        is_user_custom: true,
        ..app(id)
      }])
      .unwrap(),
    )
    .unwrap();
  };
  fs::write(a.instruction_file(NOTE), "# rules from a\n").unwrap();
  write_agents(&a, "agent-a");
  backup_with(&a, &repo).unwrap();
  fs::write(b.instruction_file(NOTE), "# rules from b\n").unwrap();
  write_agents(&b, "agent-b");

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![
      (instruction_key(NOTE), Resolution::Conflict),
      (AGENTS_KEY.to_string(), Resolution::Conflict),
    ]
  );
  // Agents cannot be kept twice.
  assert!(apply_restore_with(
    &b,
    &repo,
    &planned,
    &decide(&[
      (&instruction_key(NOTE), ConflictChoice::KeepLocal),
      (AGENTS_KEY, ConflictChoice::KeepBoth),
    ]),
    &RestoreOptions::default(),
    InstallMode::Copy,
  )
  .is_err());
  let outcome = apply(
    &b,
    &repo,
    &planned,
    decide(&[
      (&instruction_key(NOTE), ConflictChoice::KeepLocal),
      (AGENTS_KEY, ConflictChoice::TakeRemote),
    ]),
  );
  assert!(outcome.agents_changed);
  assert!(outcome.pushed);
  assert_eq!(
    fs::read_to_string(b.instruction_file(NOTE)).unwrap(),
    "# rules from b\n"
  );
  assert!(read_user_agent_apps(&b).unwrap()[0].id == "agent-a");
  assert_eq!(
    repo.head_file(&format!("instructions/{}.md", NOTE)),
    Some(b"# rules from b\n".to_vec())
  );
}

#[test]
fn a_difference_only_in_agents_never_conflicts() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code", "codex"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  let b = computer(tmp.path(), "b", &["claude-code", "codex"]);
  pull(&b, &repo);

  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();
  install(&b, FOO, &["codex"]);

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(planned.items.is_empty(), "{:?}", planned.items);
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(outcome.pushed);
  assert_eq!(
    library_at_head(&repo).skills[FOO].agents,
    vec!["claude-code", "codex"]
  );
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn keep_both_keeps_the_repository_id_and_adds_a_detached_copy() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    set_source(a, FOO, github_source("skills/foo"));
  });
  install_as(&b, FOO, &["claude-code"], InstallMode::Symlink);
  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, FOO, "foo", "from b");

  let planned = plan(&b, &repo, RestoreMode::Pull);
  let outcome = apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(FOO), ConflictChoice::KeepBoth)]),
  );
  assert!(outcome.pushed);
  let lock = store(&b).read().unwrap();
  assert_eq!(lock.skills.len(), 2);
  assert!(skill_body(&b, FOO, "foo").contains("from a"));
  assert_eq!(
    lock.skills[FOO].installs.len(),
    1,
    "installs stay with the id"
  );
  let (copy_id, copy) = lock
    .skills
    .iter()
    .find(|(id, _)| id.as_str() != FOO)
    .unwrap();
  assert_eq!(copy.name, "foo-local");
  assert_eq!(copy.source, SkillSource::None);
  assert!(copy.installs.is_empty());
  let body = skill_body(&b, copy_id, "foo-local");
  assert!(body.contains("name: foo-local") && body.contains("from b"));
  let head = library_at_head(&repo);
  assert_eq!(head.skills[copy_id.as_str()].name, "foo-local");
  assert_eq!(
    head.skills[FOO].source.key(),
    github_source("skills/foo").key()
  );
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    hub_service::get_hub_skill(&b, copy_id)
      .unwrap()
      .unwrap()
      .hub_state,
    crate::models::HubState::Ok
  );
}

#[test]
fn first_merge_pairs_equal_content_and_keeps_installs() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  add_skill(&a, BAZ, "shared", "repo version");
  set_source(&a, BAZ, github_source("skills/shared"));
  backup_with(&a, &repo).unwrap();

  // B imported the same skill (another id), the same source with other content, and an
  // unrelated skill.
  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "foo", "v1");
  install_as(&b, BAR, &["claude-code"], InstallMode::Symlink);
  const SHARED_B: &str = "00000000-0000-4000-8000-0000000000b1";
  const OWN_B: &str = "00000000-0000-4000-8000-0000000000b2";
  add_skill(&b, SHARED_B, "shared", "local version");
  set_source(&b, SHARED_B, github_source("skills/shared"));
  add_skill(&b, OWN_B, "own", "v1");

  let planned = plan(&b, &repo, RestoreMode::Merge);
  let foo = item(&planned, &skill_key(FOO));
  assert_eq!(
    (foo.local_id.as_deref(), foo.match_reason, foo.resolution),
    (Some(BAR), Some(MatchReason::Hash), Resolution::TakeRemote)
  );
  let shared = item(&planned, &skill_key(BAZ));
  assert_eq!(
    (
      shared.local_id.as_deref(),
      shared.match_reason,
      shared.resolution
    ),
    (
      Some(SHARED_B),
      Some(MatchReason::SourceKey),
      Resolution::Conflict
    )
  );
  assert_eq!(
    item(&planned, &skill_key(OWN_B)).resolution,
    Resolution::KeepLocal
  );

  apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(BAZ), ConflictChoice::TakeRemote)]),
  );
  let lock = store(&b).read().unwrap();
  assert_eq!(
    lock.skills.keys().cloned().collect::<BTreeSet<_>>(),
    BTreeSet::from([FOO.to_string(), BAZ.to_string(), OWN_B.to_string()])
  );
  let install = &lock.skills[FOO].installs[0];
  assert_eq!(
    fs::read_link(&install.path).unwrap(),
    b.hub_dir(FOO, "foo"),
    "the symlink install follows the rekey"
  );
  assert!(skill_body(&b, BAZ, "shared").contains("repo version"));
  assert!(!b.hub_root.join(BAR).exists());
  assert!(library_at_head(&repo).skills.contains_key(OWN_B));
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn first_connect_can_replace_either_side() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "repo");
  backup_with(&a, &repo).unwrap();

  // Replace local: the repository wins, local-only items are removal candidates.
  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, FOO, "foo", "local");
  add_skill(&b, BAR, "bar", "local only");
  let planned = plan(&b, &repo, RestoreMode::ReplaceLocal);
  assert_eq!(
    resolutions(&planned),
    vec![
      (skill_key(BAR), Resolution::RemovalCandidate),
      (skill_key(FOO), Resolution::TakeRemote),
    ]
  );
  apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      removals: BTreeMap::from([(skill_key(BAR), RemovalChoice::Remove)]),
      ..Default::default()
    },
  );
  assert!(skill_body(&b, FOO, "foo").contains("repo"));
  assert!(!store(&b).read().unwrap().skills.contains_key(BAR));

  // Replace remote: local ids stay and the repository's other items go.
  let c = computer(tmp.path(), "c", &["claude-code"]);
  add_skill(&c, BAZ, "foo", "repo");
  let planned = plan(&c, &repo, RestoreMode::ReplaceRemote);
  assert!(planned
    .items
    .iter()
    .all(|item| item.resolution == Resolution::KeepLocal));
  let outcome = apply(&c, &repo, &planned, RestoreDecisions::default());
  assert!(outcome.pushed);
  let head = library_at_head(&repo);
  assert_eq!(
    head.skills.keys().cloned().collect::<Vec<_>>(),
    vec![BAZ.to_string()]
  );
  assert!(store(&c).read().unwrap().skills.contains_key(BAZ));
  assert!(status(&c).unwrap().changes.is_empty());
}

#[test]
fn excluded_items_are_deferred_until_taken_or_changed() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "v2");
  add_skill(&a, BAR, "bar", "v1");
  backup_with(&a, &repo).unwrap();

  let planned = plan(&b, &repo, RestoreMode::Pull);
  apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      include: BTreeMap::from([(skill_key(FOO), false), (skill_key(BAR), false)]),
      ..Default::default()
    },
  );
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
  assert!(!store(&b).read().unwrap().skills.contains_key(BAR));
  assert!(status(&b).unwrap().changes.is_empty());

  // A backup of another change keeps the deferred items in the repository unchanged.
  let before = repo.files_at(&repo.head_commit(), "skills/");
  add_skill(&b, BAZ, "baz", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  let after = repo.files_at(&repo.head_commit(), "skills/");
  for (path, file) in &before {
    assert_eq!(after.get(path), Some(file), "{}", path);
  }

  // Still offered, excluded by default.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(planned.items.iter().all(|item| item.deferred));
  assert_eq!(planned.items.len(), 2);

  // A newer change in the repository plans it again, included by default.
  pull(&a, &repo);
  edit_skill(&a, FOO, "foo", "v3");
  backup_with(&a, &repo).unwrap();
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(!item(&planned, &skill_key(FOO)).deferred);
  assert!(item(&planned, &skill_key(BAR)).deferred);
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(!outcome.pushed);
  assert!(skill_body(&b, FOO, "foo").contains("v3"));
  assert_eq!(
    read_state(&b)
      .unwrap()
      .deferred
      .keys()
      .cloned()
      .collect::<Vec<_>>(),
    vec![skill_key(BAR)]
  );
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn an_earlier_commit_is_restored_as_a_new_commit() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  let first = backup_with(&a, &repo).unwrap().commit.unwrap();
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();
  add_skill(&a, BAR, "bar", "v1");
  let last = backup_with(&a, &repo).unwrap().commit.unwrap();

  let history = history_with(&a, &repo).unwrap();
  assert_eq!(history[0].id, last);
  assert!(history.iter().any(|commit| commit.id == first));
  assert!(history[0].message.starts_with("Backup from a"));

  let planned = plan_restore_with(&a, &repo, RestoreMode::Pull, Some(&first)).unwrap();
  assert_eq!(planned.target_commit.as_deref(), Some(first.as_str()));
  assert_eq!(
    resolutions(&planned),
    vec![
      (skill_key(BAR), Resolution::RemovalCandidate),
      (skill_key(FOO), Resolution::TakeRemote),
    ]
  );
  let outcome = apply(&a, &repo, &planned, RestoreDecisions::default());
  assert!(outcome.pushed);
  let head = repo.head_commit();
  assert_eq!(repo.parent_of(&head).as_deref(), Some(last.as_str()));
  assert_eq!(
    repo.files_at(&head, &format!("skills/{}/", FOO)),
    repo.files_at(&first, &format!("skills/{}/", FOO))
  );
  assert!(library_at_head(&repo).skills.contains_key(BAR));
  assert!(skill_body(&a, FOO, "foo").contains("v1"));
  assert!(status(&a).unwrap().changes.is_empty());
}

#[test]
fn a_stale_plan_writes_nothing() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();

  // The repository moved.
  let checkpoint = status(&b).unwrap().checkpoint;
  let planned = plan(&b, &repo, RestoreMode::Pull);
  add_skill(&a, BAR, "bar", "v1");
  backup_with(&a, &repo).unwrap();
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert_eq!(outcome.result, RestoreResult::Stale);
  assert!(!outcome.local_applied);
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
  assert_eq!(status(&b).unwrap().checkpoint, checkpoint);

  // This computer changed.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  add_skill(&b, BAZ, "baz", "v1");
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert_eq!(outcome.result, RestoreResult::Stale);
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
}

#[test]
fn undo_puts_back_the_library_before_the_restore() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  add_skill(&a, BAZ, "baz", "repo");
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "foo", "v1");
  install_as(&b, BAR, &["claude-code"], InstallMode::Symlink);
  add_skill(&b, BAZ, "baz", "local");
  let files = |env: &Env| {
    [
      env.lock_path.clone(),
      env.instruction_lock_path.clone(),
      state_path(env),
    ]
    .map(|path| fs::read(path).ok())
  };
  let before = files(&b);
  let baz_before = hub_hash(&b, BAZ, "baz");

  let planned = plan(&b, &repo, RestoreMode::Merge);
  apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(BAZ), ConflictChoice::TakeRemote)]),
  );
  assert!(store(&b).read().unwrap().skills.contains_key(FOO));
  assert!(status(&b).unwrap().checkpoint.is_some());

  undo_restore(&b).unwrap();
  assert_eq!(files(&b), before);
  let lock = store(&b).read().unwrap();
  assert!(lock.skills.contains_key(BAR) && !lock.skills.contains_key(FOO));
  assert!(!b.hub_root.join(FOO).exists());
  assert_eq!(hub_hash(&b, BAZ, "baz"), baz_before);
  assert_eq!(
    fs::read_link(&lock.skills[BAR].installs[0].path).unwrap(),
    b.hub_dir(BAR, "foo")
  );
  assert!(status(&b).unwrap().checkpoint.is_none());
  assert!(undo_restore(&b).is_err());
}

#[test]
fn restore_can_apply_the_installs() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  let planned = plan(&b, &repo, RestoreMode::Pull);
  let outcome = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions {
      apply_installs: true,
    },
    InstallMode::Copy,
  )
  .unwrap();
  let installs = outcome.installs.unwrap();
  assert_eq!(installs.applied, 1, "{:?}", installs.failures);
  assert!(b.home.join(".claude-code/skills/foo/SKILL.md").is_file());
  assert!(pending(&b).unwrap().is_empty());
}

#[test]
fn a_failed_download_changes_nothing_and_keeps_the_last_checkpoint() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();

  let base = status_commit(&b);
  let checkpoint = status(&b).unwrap().checkpoint;
  assert!(checkpoint.is_some());
  let planned = plan(&b, &repo, RestoreMode::Pull);
  repo.fail_downloads();
  let failed = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions::default(),
    InstallMode::Copy,
  );
  assert!(failed.is_err());
  assert_eq!(status_commit(&b), base);
  assert_eq!(status(&b).unwrap().checkpoint, checkpoint);
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
}

#[test]
fn a_repository_item_without_files_stops_the_restore_before_any_write() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "v2");
  add_skill(&a, BAR, "bar", "v1");
  backup_with(&a, &repo).unwrap();
  // A commit whose library lists BAR but holds none of its files.
  let head = repo.head_commit();
  let mut files = repo.files_at(&head, "");
  files.retain(|path, _| !path.starts_with(&format!("skills/{}/", BAR)));
  let tree = repo.create_tree(&files).unwrap();
  let broken = repo.create_commit("broken", &tree, Some(&head)).unwrap();
  repo.update_head(&broken).unwrap();

  let base = status_commit(&b);
  let planned = plan(&b, &repo, RestoreMode::Pull);
  let failed = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions::default(),
    InstallMode::Copy,
  );
  assert!(failed.unwrap_err().contains("no files for bar"));
  assert_eq!(status_commit(&b), base);
  assert_eq!(repo.head_commit(), broken);
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
}

#[cfg(unix)]
#[test]
fn a_failed_removal_keeps_the_base_and_can_be_undone() {
  use std::os::unix::fs::PermissionsExt;
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  install_as(&b, FOO, &["claude-code"], InstallMode::Symlink);
  hub_service::remove_hub_skill(&a, FOO, false).unwrap();
  backup_with(&a, &repo).unwrap();

  let base = status_commit(&b);
  let head = repo.head_commit();
  let planned = plan(&b, &repo, RestoreMode::Pull);
  // The symlink install cannot become a copy in a read-only directory.
  let skills_dir = b.home.join(".claude-code/skills");
  fs::set_permissions(&skills_dir, fs::Permissions::from_mode(0o555)).unwrap();
  let failed = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions {
      removals: BTreeMap::from([(skill_key(FOO), RemovalChoice::Remove)]),
      ..Default::default()
    },
    &RestoreOptions::default(),
    InstallMode::Copy,
  );
  fs::set_permissions(&skills_dir, fs::Permissions::from_mode(0o755)).unwrap();
  assert!(failed.is_err());
  assert_eq!(status_commit(&b), base);
  assert_eq!(repo.head_commit(), head, "nothing was pushed");
  assert_eq!(
    status(&b).unwrap().checkpoint.map(|info| info.target),
    Some(head)
  );

  undo_restore(&b).unwrap();
  let lock = store(&b).read().unwrap();
  assert!(lock.skills.contains_key(FOO));
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
  assert_eq!(
    fs::read_link(&lock.skills[FOO].installs[0].path).unwrap(),
    b.hub_dir(FOO, "foo")
  );
}

#[test]
fn different_sources_for_the_same_files_conflict() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  set_source(&a, FOO, github_source("skills/foo"));
  backup_with(&a, &repo).unwrap();
  set_source(&b, FOO, github_source("skills/other"));

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![(skill_key(FOO), Resolution::Conflict)]
  );

  // The same source on both sides is the same change.
  set_source(&b, FOO, github_source("skills/foo"));
  assert!(plan(&b, &repo, RestoreMode::Pull).items.is_empty());
}

const SHARED_B: &str = "00000000-0000-4000-8000-0000000000b1";

/// A has `shared` (BAZ) from a GitHub folder; B imported the same folder as `local_name`
/// with other content and a symlink install.
fn first_merge_pair(root: &Path, repo: &FakeRepo, local_name: &str) -> Env {
  let a = computer(root, "a", &["claude-code"]);
  add_skill(&a, BAZ, "shared", "repo version");
  set_source(&a, BAZ, github_source("skills/shared"));
  backup_with(&a, repo).unwrap();
  let b = computer(root, "b", &["claude-code"]);
  add_skill(&b, SHARED_B, local_name, "local version");
  set_source(&b, SHARED_B, github_source("skills/shared"));
  install_as(&b, SHARED_B, &["claude-code"], InstallMode::Symlink);
  b
}

#[test]
fn keep_both_on_a_first_merge_pair_adds_a_suffixed_local_copy() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let b = first_merge_pair(tmp.path(), &repo, "shared");

  let planned = plan(&b, &repo, RestoreMode::Merge);
  assert_eq!(
    item(&planned, &skill_key(BAZ)).match_reason,
    Some(MatchReason::SourceKey)
  );
  let outcome = apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(BAZ), ConflictChoice::KeepBoth)]),
  );
  assert!(outcome.pushed);
  let lock = store(&b).read().unwrap();
  assert_eq!(lock.skills.len(), 2);
  assert!(!lock.skills.contains_key(SHARED_B));
  assert!(skill_body(&b, BAZ, "shared").contains("repo version"));
  let install = &lock.skills[BAZ].installs[0];
  assert_eq!(
    fs::read_link(&install.path).unwrap(),
    b.hub_dir(BAZ, "shared")
  );
  let (copy_id, copy) = lock
    .skills
    .iter()
    .find(|(id, _)| id.as_str() != BAZ)
    .unwrap();
  assert_eq!(copy.name, "shared-local");
  assert_eq!(copy.source, SkillSource::None);
  assert!(copy.installs.is_empty());
  assert!(skill_body(&b, copy_id, "shared-local").contains("local version"));
  let head = library_at_head(&repo);
  assert_eq!(head.skills[copy_id.as_str()].name, "shared-local");
  assert_eq!(head.skills[BAZ].name, "shared");
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn symlink_installs_follow_a_rekey_and_a_rename() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let b = first_merge_pair(tmp.path(), &repo, "mine");

  let planned = plan(&b, &repo, RestoreMode::Merge);
  apply(
    &b,
    &repo,
    &planned,
    decide(&[(&skill_key(BAZ), ConflictChoice::TakeRemote)]),
  );
  let lock = store(&b).read().unwrap();
  assert_eq!(lock.skills[BAZ].name, "shared");
  let link = PathBuf::from(&lock.skills[BAZ].installs[0].path);
  assert_eq!(fs::read_link(&link).unwrap(), b.hub_dir(BAZ, "shared"));
  assert!(link.join("SKILL.md").is_file());
}

#[test]
fn undo_takes_back_the_installs_of_the_restore() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  let planned = plan(&b, &repo, RestoreMode::Pull);
  let outcome = apply_restore_with(
    &b,
    &repo,
    &planned,
    &RestoreDecisions::default(),
    &RestoreOptions {
      apply_installs: true,
    },
    InstallMode::Copy,
  )
  .unwrap();
  assert_eq!(outcome.installs.unwrap().applied, 1);
  let target = b.home.join(".claude-code/skills/foo");
  assert!(target.join("SKILL.md").is_file());

  undo_restore(&b).unwrap();
  assert!(!target.exists());
  assert!(store(&b).read().unwrap().skills.is_empty());
  assert!(status(&b).unwrap().checkpoint.is_none());
}

#[test]
fn undo_refuses_to_discard_a_later_edit() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (_, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&b, FOO, "foo", "edited after the restore");

  let err = undo_restore(&b).unwrap_err();
  assert!(
    err.contains(&b.hub_root.join(FOO).to_string_lossy().to_string()),
    "{}",
    err
  );
  assert!(skill_body(&b, FOO, "foo").contains("edited after the restore"));
  assert!(status(&b).unwrap().checkpoint.is_some());
}

#[cfg(unix)]
#[test]
fn undo_keeps_the_checkpoint_until_every_link_is_back() {
  use std::os::unix::fs::PermissionsExt;
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "foo", "v1");
  install_as(&b, BAR, &["claude-code"], InstallMode::Symlink);
  let planned = plan(&b, &repo, RestoreMode::Merge);
  apply(&b, &repo, &planned, RestoreDecisions::default());

  let skills_dir = b.home.join(".claude-code/skills");
  fs::set_permissions(&skills_dir, fs::Permissions::from_mode(0o555)).unwrap();
  let failed = undo_restore(&b);
  fs::set_permissions(&skills_dir, fs::Permissions::from_mode(0o755)).unwrap();
  assert!(failed.is_err());
  assert!(status(&b).unwrap().checkpoint.is_some());

  undo_restore(&b).unwrap();
  let lock = store(&b).read().unwrap();
  assert!(lock.skills.contains_key(BAR));
  assert_eq!(
    fs::read_link(&lock.skills[BAR].installs[0].path).unwrap(),
    b.hub_dir(BAR, "foo")
  );
  assert!(status(&b).unwrap().checkpoint.is_none());
}
