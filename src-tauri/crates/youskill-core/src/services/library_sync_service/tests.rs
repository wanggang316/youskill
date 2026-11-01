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
  write_custom_agents(&a, &[custom_agent("my-agent", "My agent")]);

  let backup = backup_with(&a, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  // The skill, the instruction, the custom agent and the device file.
  assert_eq!(backup.changes.len(), 4, "{:?}", backup.changes);
  assert!(status(&a).unwrap().changes.is_empty());

  let b = computer(tmp.path(), "b", &["claude-code", "codex"]);
  assert!(status(&b).unwrap().library_empty);
  let pulled = pull(&b, &repo);
  assert_eq!(pulled.result, RestoreResult::Applied);
  assert!(pulled.pushed, "the new device file goes to the repository");
  assert!(pulled.agents_changed);
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert_eq!(
    fs::read(b.instruction_file(NOTE)).unwrap(),
    b"# rules\n".to_vec()
  );
  assert_eq!(read_user_agent_apps(&b).unwrap()[0].id, "my-agent");
  assert!(status(&b).unwrap().changes.is_empty());
  // The installs of A stay on A.
  assert!(pending(&b).unwrap().is_empty());
  assert!(device_file(&repo, &b).installs.is_empty());
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
  let installed = backup_with(&a, &repo).unwrap().commit.unwrap();
  uninstall(&a, FOO, "claude-code");
  backup_with(&a, &repo).unwrap();

  let target = a.home.join(".claude-code/skills/foo");
  fs::create_dir_all(&target).unwrap();
  fs::write(target.join("SKILL.md"), "other").unwrap();
  let planned = plan_restore_with(&a, &repo, RestoreMode::Pull, Some(&installed)).unwrap();
  apply(&a, &repo, &planned, RestoreDecisions::default());

  let items = pending(&a).unwrap();
  let applied = apply_pending(&a, &items, InstallMode::Copy).unwrap();
  assert_eq!(applied.applied, 0);
  assert_eq!(applied.failures.len(), 1, "{:?}", applied.failures);
  assert_eq!(applied.failures[0].kind, ItemKind::Skill);
  assert_eq!(applied.failures[0].name, "foo");
  assert!(applied.failures[0].reason.contains("foo"));
  assert_eq!(
    fs::read_to_string(target.join("SKILL.md")).unwrap(),
    "other"
  );
  assert_eq!(pending(&a).unwrap(), items);
}

#[test]
fn repository_holds_no_paths_of_this_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();

  let home = a.home.to_string_lossy().to_string();
  let library = String::from_utf8(repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  assert!(!library.contains(&home));
  assert!(!library.contains("\"agents\""), "{}", library);
  assert_eq!(library_at_head(&repo).version, 2);
  assert_eq!(library_at_head(&repo).skills[FOO].scope, Scope::Shared);
  let path = device_path(&device_id(&a));
  let device = String::from_utf8(repo.head_file(&path).unwrap()).unwrap();
  assert!(!device.contains(&home));
  let device = device_file(&repo, &a);
  assert_eq!(device.name, "a");
  assert!(device.last_backup_at.is_some());
  assert_eq!(
    device.installs,
    BTreeMap::from([(
      "claude-code".to_string(),
      vec![DeviceInstall {
        kind: ItemKind::Skill,
        id: FOO.to_string()
      }]
    )])
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
fn installs_stay_on_the_device_that_made_them() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));

  install(&a, FOO, &["claude-code"]);
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert_eq!(
    device_file(&repo, &a).by_item()[&skill_key(FOO)],
    vec!["claude-code"]
  );

  // B restores: the install of A is not pending here.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(planned.items.is_empty(), "{:?}", planned.items);
  apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(pending(&b).unwrap().is_empty());
  assert!(device_file(&repo, &b).installs.is_empty());

  // An uninstall on A changes only the device file of A.
  uninstall(&a, FOO, "claude-code");
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert!(device_file(&repo, &a).installs.is_empty());
  assert!(pending(&a).unwrap().is_empty());
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
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_instruction(a, NOTE, "# rules\n");
    write_custom_agents(a, &[custom_agent("shared-agent", "Shared")]);
  });
  fs::write(a.instruction_file(NOTE), "# rules from a\n").unwrap();
  write_custom_agents(&a, &[custom_agent("shared-agent", "From a")]);
  backup_with(&a, &repo).unwrap();
  fs::write(b.instruction_file(NOTE), "# rules from b\n").unwrap();
  write_custom_agents(&b, &[custom_agent("shared-agent", "From b")]);

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![
      (instruction_key(NOTE), Resolution::Conflict),
      (agent_key("shared-agent"), Resolution::Conflict),
    ]
  );
  // Agents cannot be kept twice.
  assert!(apply_restore_with(
    &b,
    &repo,
    &planned,
    &decide(&[
      (&instruction_key(NOTE), ConflictChoice::KeepLocal),
      (&agent_key("shared-agent"), ConflictChoice::KeepBoth),
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
      (&agent_key("shared-agent"), ConflictChoice::TakeRemote),
    ]),
  );
  assert!(outcome.agents_changed);
  assert!(outcome.pushed);
  assert_eq!(
    fs::read_to_string(b.instruction_file(NOTE)).unwrap(),
    "# rules from b\n"
  );
  assert_eq!(read_user_agent_apps(&b).unwrap()[0].display_name, "From a");
  assert_eq!(
    repo.head_file(&format!("instructions/{}.md", NOTE)),
    Some(b"# rules from b\n".to_vec())
  );
}

#[test]
fn installs_on_two_computers_never_conflict() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));

  install(&a, FOO, &["claude-code"]);
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  install(&b, FOO, &["claude-code"]);
  let backup = backup_with(&b, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  assert_eq!(backup.incoming, 0);
  assert_eq!(
    device_file(&repo, &a).by_item()[&skill_key(FOO)],
    vec!["claude-code"]
  );
  assert_eq!(
    device_file(&repo, &b).by_item()[&skill_key(FOO)],
    vec!["claude-code"]
  );
  assert!(status(&b).unwrap().changes.is_empty());
  assert!(pending(&b).unwrap().is_empty());
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
  let installed = backup_with(&a, &repo).unwrap().commit.unwrap();
  uninstall(&a, FOO, "claude-code");
  backup_with(&a, &repo).unwrap();

  let planned = plan_restore_with(&a, &repo, RestoreMode::Pull, Some(&installed)).unwrap();
  let outcome = apply_restore_with(
    &a,
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
  assert!(a.home.join(".claude-code/skills/foo/SKILL.md").is_file());
  assert!(pending(&a).unwrap().is_empty());
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
  let installed = backup_with(&a, &repo).unwrap().commit.unwrap();
  uninstall(&a, FOO, "claude-code");
  backup_with(&a, &repo).unwrap();
  let state_before = fs::read(state_path(&a)).unwrap();

  let planned = plan_restore_with(&a, &repo, RestoreMode::Pull, Some(&installed)).unwrap();
  let outcome = apply_restore_with(
    &a,
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
  let target = a.home.join(".claude-code/skills/foo");
  assert!(target.join("SKILL.md").is_file());
  assert!(!read_state(&a)
    .unwrap()
    .base_device
    .unwrap()
    .installs
    .is_empty());

  undo_restore(&a).unwrap();
  assert!(!target.exists());
  assert!(store(&a).read().unwrap().skills[FOO].installs.is_empty());
  // The sync state, with the device file of the base, is back.
  assert_eq!(fs::read(state_path(&a)).unwrap(), state_before);
  assert!(pending(&a).unwrap().is_empty());
  assert!(status(&a).unwrap().checkpoint.is_none());
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

// ---------------------------------------------------------------------------
// Layout version 2: device files, item scope, backup on a moved head
// ---------------------------------------------------------------------------

fn uninstall(env: &Env, id: &str, agent: &str) {
  let result = install_service::uninstall_skill(
    env,
    UninstallRequest {
      name: id.to_string(),
      targets: vec![InstallTargetSpec {
        scope: InstallScope::User,
        project_path: None,
        agent_id: agent.to_string(),
      }],
      paths: Vec::new(),
      force: false,
    },
  )
  .unwrap();
  assert!(result.applied, "{:?}", result.blockers);
}

fn custom_agent(id: &str, display_name: &str) -> AgentApp {
  AgentApp {
    display_name: display_name.to_string(),
    is_user_custom: true,
    ..app(id)
  }
}

fn write_custom_agents(env: &Env, apps: &[AgentApp]) {
  fs::write(
    env.config_dir.join(USER_AGENT_APPS_FILE),
    serde_json::to_string(apps).unwrap(),
  )
  .unwrap();
}

fn device_id(env: &Env) -> String {
  read_state(env).unwrap().device_id.unwrap()
}

fn device_file(repo: &FakeRepo, env: &Env) -> DeviceFile {
  serde_json::from_slice(&repo.head_file(&device_path(&device_id(env))).unwrap()).unwrap()
}

fn agents_at_head(repo: &FakeRepo) -> Vec<SyncedAgent> {
  parse_agents(&repo.head_file(AGENTS_FILE).unwrap()).unwrap()
}

/// Replace the head with a commit whose files are `files` plus `extra` contents.
fn commit_files(
  repo: &FakeRepo,
  mut files: BTreeMap<String, TreeFile>,
  extra: &[(&str, Vec<u8>)],
  message: &str,
) -> String {
  for (path, content) in extra {
    let sha = repo.create_blob(content).unwrap();
    files.insert(
      path.to_string(),
      TreeFile {
        mode: MODE_FILE.to_string(),
        sha,
      },
    );
  }
  let head = repo.head_commit();
  let tree = repo.create_tree(&files).unwrap();
  let commit = repo.create_commit(message, &tree, Some(&head)).unwrap();
  repo.update_head(&commit).unwrap();
  commit
}

/// Turn the head into the version 1 layout of release 0.10.0: `agents` on each record
/// (`agents` gives them), no scope, no device files.
fn downgrade_head(repo: &FakeRepo, agents: &[(&str, &[&str])]) -> String {
  let mut library: serde_json::Value =
    serde_json::from_slice(&repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  library["version"] = 1.into();
  for kind in ["skills", "instructions"] {
    for (id, record) in library[kind].as_object_mut().unwrap().iter_mut() {
      let record = record.as_object_mut().unwrap();
      record.remove("scope");
      let enabled = agents
        .iter()
        .find(|(item, _)| item == id)
        .map(|(_, agents)| agents.to_vec())
        .unwrap_or_default();
      record.insert("agents".to_string(), serde_json::json!(enabled));
    }
  }
  let mut custom: Vec<serde_json::Value> =
    serde_json::from_slice(&repo.head_file(AGENTS_FILE).unwrap()).unwrap();
  for agent in &mut custom {
    agent.as_object_mut().unwrap().remove("scope");
  }
  let mut files = repo.files_at(&repo.head_commit(), "");
  files.retain(|path, _| !path.starts_with("devices/"));
  commit_files(
    repo,
    files,
    &[
      (LIBRARY_FILE, pretty_json(&library).unwrap()),
      (AGENTS_FILE, pretty_json(&custom).unwrap()),
    ],
    "Backup from an old app",
  )
}

/// Make `env` a computer of release 0.10.0 whose base is the head: no device id, no
/// device file and no custom agents in the sync state.
fn old_app_state(env: &Env, repo: &FakeRepo) {
  let head = repo.head_commit();
  let mut state = read_state(env).unwrap();
  state.device_id = None;
  state.last_synced_commit = Some(head.clone());
  state.base_files = repo.files_at(&head, "");
  state.base_library =
    Some(serde_json::from_slice(&repo.head_file(LIBRARY_FILE).unwrap()).unwrap());
  state.base_agents = None;
  state.base_device = None;
  state.base_local_agents = store(env)
    .read()
    .unwrap()
    .skills
    .iter()
    .filter(|(_, record)| !record.installs.is_empty())
    .map(|(id, record)| (skill_key(id), user_agents(&record.installs)))
    .collect();
  write_state(env, &state).unwrap();
}

const OTHER_DEVICE: &str = "00000000-0000-4000-8000-0000000000dd";
const OTHER_SKILL: &str = "00000000-0000-4000-8000-0000000000d1";

/// Add a device-only skill, a device-only custom agent and the device file of another
/// device to the head.
fn add_other_device(repo: &FakeRepo) {
  let mut library = library_at_head(repo);
  library.skills.insert(
    OTHER_SKILL.to_string(),
    LibrarySkill {
      name: "theirs".to_string(),
      source: SkillSource::None,
      hash: "h".to_string(),
      imported_at: "2026-10-01T00:00:00Z".to_string(),
      updated_at: "2026-10-01T00:00:00Z".to_string(),
      scope: Scope::Device(OTHER_DEVICE.to_string()),
    },
  );
  let mut agents = agents_at_head(repo);
  agents.push(SyncedAgent {
    app: custom_agent("their-agent", "Theirs"),
    scope: Scope::Device(OTHER_DEVICE.to_string()),
  });
  let other = DeviceFile {
    name: "other".to_string(),
    installs: BTreeMap::from([(
      "claude-code".to_string(),
      vec![DeviceInstall {
        kind: ItemKind::Skill,
        id: OTHER_SKILL.to_string(),
      }],
    )]),
    last_backup_at: None,
  };
  commit_files(
    repo,
    repo.files_at(&repo.head_commit(), ""),
    &[
      (LIBRARY_FILE, pretty_json(&library).unwrap()),
      (AGENTS_FILE, pretty_json(&agents).unwrap()),
      (
        &format!("skills/{}/theirs/SKILL.md", OTHER_SKILL),
        b"theirs".to_vec(),
      ),
      (&device_path(OTHER_DEVICE), pretty_json(&other).unwrap()),
    ],
    "Backup from other",
  );
}

/// The parts of the other device are in the head unchanged.
fn assert_other_device_kept(repo: &FakeRepo) {
  let library = library_at_head(repo);
  assert_eq!(
    library.skills[OTHER_SKILL].scope,
    Scope::Device(OTHER_DEVICE.to_string())
  );
  assert_eq!(
    repo.head_file(&format!("skills/{}/theirs/SKILL.md", OTHER_SKILL)),
    Some(b"theirs".to_vec())
  );
  assert!(agents_at_head(repo)
    .iter()
    .any(|agent| agent.app.id == "their-agent"));
  let other: DeviceFile =
    serde_json::from_slice(&repo.head_file(&device_path(OTHER_DEVICE)).unwrap()).unwrap();
  assert_eq!(other.name, "other");
}

fn assert_not_here(env: &Env) {
  assert!(!store(env).read().unwrap().skills.contains_key(OTHER_SKILL));
  assert!(!env.hub_root.join(OTHER_SKILL).exists());
  assert!(read_user_agent_apps(env)
    .unwrap()
    .iter()
    .all(|app| app.id != "their-agent"));
}

#[test]
fn device_gets_shared_items_and_its_own_items_only() {
  assert!(device_gets(&Scope::Shared, "me"));
  assert!(device_gets(&Scope::Device("me".to_string()), "me"));
  assert!(!device_gets(&Scope::Device("other".to_string()), "me"));
  assert!(!device_gets(&Scope::Device("other".to_string()), ""));
}

#[test]
fn a_library_above_this_version_is_refused() {
  // Releases up to 0.10.0 refuse a library above version 1; this one refuses above 2.
  const { assert!(LIBRARY_VERSION > 1) };
  let newer = LibraryFile {
    version: LIBRARY_VERSION + 1,
    ..LibraryFile::default()
  };
  let err = restore::parse_library(&pretty_json(&newer).unwrap()).unwrap_err();
  assert!(err.contains("newer YouSkill"), "{}", err);
  let current = LibraryFile::default();
  assert!(restore::parse_library(&pretty_json(&current).unwrap()).is_ok());
}

#[test]
fn migration_writes_one_commit_with_shared_items_and_this_device_file() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code", "codex"]);
  add_skill(&a, FOO, "foo", "v1");
  install(&a, FOO, &["claude-code"]);
  write_custom_agents(&a, &[custom_agent("my-agent", "Mine")]);
  backup_with(&a, &repo).unwrap();
  // The old union of all devices enables codex too; this computer only has claude-code.
  let old = downgrade_head(&repo, &[(FOO, &["claude-code", "codex"])]);
  old_app_state(&a, &repo);

  assert_eq!(
    backup_with(&a, &repo).unwrap().result,
    BackupResult::MigrationRequired
  );
  assert_eq!(
    plan(&a, &repo, RestoreMode::Pull).state,
    PlanState::MigrationRequired
  );
  assert_eq!(
    repo.head_commit(),
    old,
    "nothing is written before the migration"
  );

  assert!(status(&a).unwrap().legacy_base);

  let commits = repo.commit_count();
  migrate_with(&a, &repo).unwrap();
  assert!(!status(&a).unwrap().legacy_base);
  assert_eq!(repo.commit_count(), commits + 1);
  let head = repo.head_commit();
  assert_eq!(repo.parent_of(&head), Some(old));
  let text = String::from_utf8(repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  assert!(!text.contains("\"agents\""), "{}", text);
  let library = library_at_head(&repo);
  assert_eq!(library.version, 2);
  assert_eq!(library.skills[FOO].scope, Scope::Shared);
  assert_eq!(agents_at_head(&repo)[0].scope, Scope::Shared);
  assert_eq!(
    device_file(&repo, &a).by_item(),
    BTreeMap::from([(skill_key(FOO), vec!["claude-code".to_string()])])
  );

  // The migration commit is the base: nothing more to back up, nothing pending.
  assert_eq!(status_commit(&a), Some(head.clone()));
  assert!(status(&a).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&a, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  assert!(pending(&a).unwrap().is_empty());
  // A second migration does nothing.
  migrate_with(&a, &repo).unwrap();
  assert_eq!(repo.head_commit(), head);
}

#[test]
fn a_computer_with_a_version_1_base_syncs_after_the_migration() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
    write_custom_agents(a, &[custom_agent("my-agent", "Mine")]);
  });
  install(&b, FOO, &["claude-code"]);
  downgrade_head(&repo, &[(FOO, &["claude-code"])]);
  old_app_state(&a, &repo);
  old_app_state(&b, &repo);

  migrate_with(&a, &repo).unwrap();
  let migrated = repo.head_commit();

  // B reads its version 1 base as version 2: no false changes, only its new device file.
  let changes = status(&b).unwrap().changes;
  assert_eq!(
    changes.iter().map(|c| c.kind).collect::<Vec<_>>(),
    vec![ItemKind::Device],
    "{:?}",
    changes
  );
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(planned.items.is_empty(), "{:?}", planned.items);
  let backup = backup_with(&b, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  assert_eq!(backup.incoming, 0);
  assert_eq!(repo.parent_of(&repo.head_commit()), Some(migrated));
  assert!(read_state(&b).unwrap().device_id.is_some());
  assert_eq!(
    device_file(&repo, &b).by_item(),
    BTreeMap::from([(skill_key(FOO), vec!["claude-code".to_string()])])
  );
  assert!(device_file(&repo, &a).installs.is_empty());
  assert_eq!(library_at_head(&repo).skills.len(), 2);
  assert_eq!(agents_at_head(&repo).len(), 1);
  assert!(pending(&b).unwrap().is_empty());
}

#[test]
fn migration_on_a_moved_head_keeps_the_changes_of_the_other_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  install(&b, FOO, &["claude-code"]);
  downgrade_head(&repo, &[(FOO, &["claude-code"])]);
  old_app_state(&a, &repo);
  old_app_state(&b, &repo);

  // An old app on A backs up BAR after B's last sync.
  let mut library: serde_json::Value =
    serde_json::from_slice(&repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  library["skills"][BAR] = serde_json::json!({
    "name": "bar", "hash": "h", "importedAt": "t", "updatedAt": "t", "agents": []
  });
  commit_files(
    &repo,
    repo.files_at(&repo.head_commit(), ""),
    &[
      (LIBRARY_FILE, pretty_json(&library).unwrap()),
      (&format!("skills/{}/bar/SKILL.md", BAR), b"bar".to_vec()),
    ],
    "Backup from an old app",
  );

  migrate_with(&b, &repo).unwrap();
  let migrated = repo.head_commit();
  assert_eq!(status_commit(&b), Some(migrated.clone()));
  assert!(read_state(&b).unwrap().last_backup_at.is_some());
  assert!(
    read_state(&b)
      .unwrap()
      .incoming
      .contains_key(&skill_key(BAR)),
    "BAR is kept for the next restore"
  );
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  assert_eq!(
    repo.head_commit(),
    migrated,
    "no commit after the migration"
  );

  let planned = plan(&b, &repo, RestoreMode::Pull);
  let bar = item(&planned, &skill_key(BAR));
  assert_eq!(bar.resolution, Resolution::TakeRemote);
  assert!(!bar.deferred);
}

#[test]
fn first_connect_to_a_version_1_repository_asks_for_the_migration() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  downgrade_head(&repo, &[(FOO, &["claude-code"])]);

  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "bar", "v1");
  assert_eq!(
    plan(&b, &repo, RestoreMode::Merge).state,
    PlanState::MigrationRequired
  );
  migrate_with(&b, &repo).unwrap();
  assert_eq!(library_at_head(&repo).version, 2);
  assert!(
    status_commit(&b).is_none(),
    "the first connect still has no base"
  );

  let planned = plan(&b, &repo, RestoreMode::Merge);
  assert_eq!(planned.state, PlanState::Ready);
  apply(&b, &repo, &planned, RestoreDecisions::default());
  let library = library_at_head(&repo);
  assert!(library.skills.contains_key(FOO) && library.skills.contains_key(BAR));
  assert!(store(&b).read().unwrap().skills.contains_key(FOO));
}

#[test]
fn backup_on_a_moved_head_keeps_the_changes_of_the_other_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
  });
  edit_skill(&a, FOO, "foo", "from a");
  install(&a, FOO, &["claude-code"]);
  backup_with(&a, &repo).unwrap();
  let a_files = repo.files_at(&repo.head_commit(), &format!("skills/{}/", FOO));

  edit_skill(&b, BAR, "bar", "from b");
  let backup = backup_with(&b, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  assert_eq!(backup.incoming, 1);
  assert!(
    skill_body(&b, FOO, "foo").contains("v1"),
    "not written here"
  );
  let head = repo.head_commit();
  assert_eq!(repo.files_at(&head, &format!("skills/{}/", FOO)), a_files);
  assert!(String::from_utf8(
    repo
      .head_file(&format!("skills/{}/bar/SKILL.md", BAR))
      .unwrap()
  )
  .unwrap()
  .contains("from b"));
  assert_eq!(
    device_file(&repo, &a).by_item()[&skill_key(FOO)],
    vec!["claude-code"]
  );
  assert!(status(&b).unwrap().changes.is_empty());

  // A second backup does not revert the change of A.
  add_skill(&b, BAZ, "baz", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert_eq!(
    repo.files_at(&repo.head_commit(), &format!("skills/{}/", FOO)),
    a_files
  );

  // The next restore offers it, selected by default.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  let foo = item(&planned, &skill_key(FOO));
  assert_eq!(foo.resolution, Resolution::TakeRemote);
  assert!(!foo.deferred);
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(!outcome.pushed);
  assert_eq!(hub_hash(&a, FOO, "foo"), hub_hash(&b, FOO, "foo"));
  assert!(read_state(&b).unwrap().incoming.is_empty());
  assert!(status(&b).unwrap().changes.is_empty());
  assert!(pending(&b).unwrap().is_empty());
}

#[test]
fn an_incoming_item_edited_here_is_a_conflict() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
  });
  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, BAR, "bar", "from b");
  backup_with(&b, &repo).unwrap();

  edit_skill(&b, FOO, "foo", "from b");
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![(skill_key(FOO), Resolution::Conflict)]
  );
}

#[test]
fn backup_with_a_change_on_both_sides_writes_nothing() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "from a");
  backup_with(&a, &repo).unwrap();
  edit_skill(&b, FOO, "foo", "from b");

  let head = repo.head_commit();
  let commits = repo.commit_count();
  let state = fs::read(state_path(&b)).unwrap();
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::Diverged
  );
  assert_eq!(repo.head_commit(), head);
  assert_eq!(repo.commit_count(), commits);
  assert_eq!(fs::read(state_path(&b)).unwrap(), state);
}

#[test]
fn a_removal_on_a_moved_head_is_kept_and_offered() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    add_skill(a, BAR, "bar", "v1");
  });
  hub_service::remove_hub_skill(&a, FOO, false).unwrap();
  backup_with(&a, &repo).unwrap();

  edit_skill(&b, BAR, "bar", "from b");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert!(!library_at_head(&repo).skills.contains_key(FOO));
  assert!(store(&b).read().unwrap().skills.contains_key(FOO));
  add_skill(&b, BAZ, "baz", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert!(!library_at_head(&repo).skills.contains_key(FOO));

  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![(skill_key(FOO), Resolution::RemovalCandidate)]
  );
  apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      removals: BTreeMap::from([(skill_key(FOO), RemovalChoice::Remove)]),
      ..Default::default()
    },
  );
  assert!(!store(&b).read().unwrap().skills.contains_key(FOO));
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn a_deferred_item_changed_again_on_a_moved_head_becomes_incoming() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();
  let planned = plan(&b, &repo, RestoreMode::Pull);
  apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      include: BTreeMap::from([(skill_key(FOO), false)]),
      ..Default::default()
    },
  );
  pull(&a, &repo);
  edit_skill(&a, FOO, "foo", "v3");
  backup_with(&a, &repo).unwrap();

  add_skill(&b, BAR, "bar", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  let state = read_state(&b).unwrap();
  assert!(state.deferred.is_empty());
  assert!(state.incoming.contains_key(&skill_key(FOO)));
  assert!(String::from_utf8(
    repo
      .head_file(&format!("skills/{}/foo/SKILL.md", FOO))
      .unwrap()
  )
  .unwrap()
  .contains("v3"));
  assert!(!item(&plan(&b, &repo, RestoreMode::Pull), &skill_key(FOO)).deferred);
}

#[test]
fn custom_agents_merge_one_by_one() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| add_skill(a, FOO, "foo", "v1"));
  write_custom_agents(&a, &[custom_agent("agent-a", "A")]);
  backup_with(&a, &repo).unwrap();
  write_custom_agents(&b, &[custom_agent("agent-b", "B")]);

  let backup = backup_with(&b, &repo).unwrap();
  assert_eq!(backup.result, BackupResult::Pushed);
  assert_eq!(
    agents_at_head(&repo)
      .iter()
      .map(|agent| agent.app.id.as_str())
      .collect::<Vec<_>>(),
    vec!["agent-a", "agent-b"]
  );
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert_eq!(
    resolutions(&planned),
    vec![(agent_key("agent-a"), Resolution::TakeRemote)]
  );
  let outcome = apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(outcome.agents_changed);
  assert_eq!(
    read_user_agent_apps(&b)
      .unwrap()
      .iter()
      .map(|app| app.id.as_str())
      .collect::<Vec<_>>(),
    vec!["agent-a", "agent-b"]
  );
}

#[test]
fn the_parts_of_other_devices_survive_every_sync() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  let first = backup_with(&a, &repo).unwrap().commit.unwrap();
  add_other_device(&repo);

  // Backup on the moved head.
  edit_skill(&a, FOO, "foo", "v2");
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert_other_device_kept(&repo);
  let planned = plan(&a, &repo, RestoreMode::Pull);
  assert!(planned.items.is_empty(), "{:?}", planned.items);

  // Backup on its own base.
  add_skill(&a, BAR, "bar", "v1");
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert_other_device_kept(&repo);

  // Restore of an earlier commit (before the other device existed).
  let planned = plan_restore_with(&a, &repo, RestoreMode::Pull, Some(&first)).unwrap();
  let outcome = apply(&a, &repo, &planned, RestoreDecisions::default());
  assert!(outcome.pushed);
  assert_other_device_kept(&repo);
  assert!(skill_body(&a, FOO, "foo").contains("v1"));
  assert_not_here(&a);

  // A new computer restores and replaces the repository.
  let b = computer(tmp.path(), "b", &["claude-code"]);
  pull(&b, &repo);
  assert_not_here(&b);
  assert_other_device_kept(&repo);
  let c = computer(tmp.path(), "c", &["claude-code"]);
  add_skill(&c, BAZ, "baz", "v1");
  let planned = plan(&c, &repo, RestoreMode::ReplaceRemote);
  apply(&c, &repo, &planned, RestoreDecisions::default());
  assert_other_device_kept(&repo);
  let library = library_at_head(&repo);
  assert!(library.skills.contains_key(BAZ) && !library.skills.contains_key(FOO));
  assert_not_here(&c);
  assert!(status(&c).unwrap().changes.is_empty());
}

#[test]
fn pending_comes_from_this_device_file_only() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  // Another device lists FOO for claude-code.
  let other = DeviceFile {
    name: "other".to_string(),
    installs: BTreeMap::from([(
      "claude-code".to_string(),
      vec![DeviceInstall {
        kind: ItemKind::Skill,
        id: FOO.to_string(),
      }],
    )]),
    last_backup_at: None,
  };
  commit_files(
    &repo,
    repo.files_at(&repo.head_commit(), ""),
    &[(&device_path(OTHER_DEVICE), pretty_json(&other).unwrap())],
    "Backup from other",
  );
  pull(&a, &repo);
  assert!(pending(&a).unwrap().is_empty());
}

#[test]
fn a_sync_state_of_version_1_is_read() {
  let tmp = tempfile::tempdir().unwrap();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  let old = serde_json::json!({
    "repo": "me/youskill-library",
    "branch": "main",
    "lastSyncedCommit": "c1",
    "baseLibrary": {
      "format": "youskill-library",
      "version": 1,
      "skills": { FOO: {
        "name": "foo", "hash": "h", "importedAt": "t", "updatedAt": "t",
        "agents": ["claude-code", "codex"]
      }}
    },
    "baseLocalAgents": { "skill:x": ["claude-code"] },
    "deferred": {
      "agents": { "entry": { "kind": "agents" }, "files": {} }
    }
  });
  fs::write(state_path(&a), serde_json::to_vec(&old).unwrap()).unwrap();
  let state = read_state(&a).unwrap();
  assert!(state.device_id.is_none());
  assert!(state.deferred.is_empty());
  let library = state.base_library.unwrap();
  assert_eq!(library.version, 1);
  assert_eq!(library.skills[FOO].scope, Scope::Shared);
}

#[test]
fn first_connect_migration_maps_the_installs_of_paired_items() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  backup_with(&a, &repo).unwrap();
  downgrade_head(&repo, &[(FOO, &["claude-code"])]);

  // B has the same skill under another id, installed for claude-code.
  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "foo", "v1");
  install(&b, BAR, &["claude-code"]);
  migrate_with(&b, &repo).unwrap();
  assert_eq!(
    device_file(&repo, &b).by_item(),
    BTreeMap::from([(skill_key(FOO), vec!["claude-code".to_string()])])
  );
}

#[test]
fn a_restore_before_this_device_joined_keeps_its_installs() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  let before = backup_with(&a, &repo).unwrap().commit.unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  pull(&b, &repo);
  install(&b, FOO, &["claude-code"]);
  edit_skill(&b, FOO, "foo", "v2");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);

  // The earlier commit has no device file of B: its content comes back, its installs stay.
  let planned = plan_restore_with(&b, &repo, RestoreMode::Pull, Some(&before)).unwrap();
  apply(&b, &repo, &planned, RestoreDecisions::default());
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
  assert!(pending(&b).unwrap().is_empty());
  assert_eq!(
    device_file(&repo, &b).by_item()[&skill_key(FOO)],
    vec!["claude-code"]
  );
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::UpToDate
  );
}

fn set_instruction_source(env: &Env, id: &str, source: InstructionSource) {
  instruction_service::update_lock(env, |lock| {
    lock.instructions.get_mut(id).unwrap().source = source;
    Ok(())
  })
  .unwrap();
}

fn skill_source(env: &Env, id: &str) -> SkillSource {
  store(env).read().unwrap().skills[id].source.clone()
}

fn instruction_source(env: &Env, id: &str) -> InstructionSource {
  instruction_service::read_lock(env).unwrap().instructions[id]
    .source
    .clone()
}

fn folder(path: &str) -> SkillSource {
  SkillSource::Folder {
    path: path.to_string(),
  }
}

fn file_source(path: &str) -> InstructionSource {
  InstructionSource::File {
    path: path.to_string(),
  }
}

#[test]
fn local_source_paths_stay_on_this_computer() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  let home = a.home.to_string_lossy().to_string();
  add_skill(&a, FOO, "foo", "v1");
  set_source(&a, FOO, folder(&format!("{}/skills/foo", home)));
  add_skill(&a, BAR, "bar", "v1");
  set_source(
    &a,
    BAR,
    SkillSource::Zip {
      path: format!("{}/bar.zip", home),
    },
  );
  add_instruction(&a, NOTE, "# rules\n");
  set_instruction_source(
    &a,
    NOTE,
    InstructionSource::Agent {
      path: format!("{}/.claude/CLAUDE.md", home),
    },
  );
  backup_with(&a, &repo).unwrap();

  for path in [LIBRARY_FILE, README_FILE] {
    let text = String::from_utf8(repo.head_file(path).unwrap()).unwrap();
    assert!(!text.contains(&home), "{}: {}", path, text);
  }
  let readme = String::from_utf8(repo.head_file(README_FILE).unwrap()).unwrap();
  assert!(readme.contains("| foo | folder |"), "{}", readme);
  assert!(readme.contains("| bar | zip |"), "{}", readme);
  // The source kind stays.
  let library = library_at_head(&repo);
  assert_eq!(library.skills[FOO].source, folder(""));
  assert_eq!(
    library.skills[BAR].source,
    SkillSource::Zip {
      path: String::new()
    }
  );
  assert_eq!(
    library.instructions[NOTE].source,
    InstructionSource::Agent {
      path: String::new()
    }
  );
  // The local lock keeps the paths and nothing is left to back up.
  assert_eq!(
    skill_source(&a, FOO),
    folder(&format!("{}/skills/foo", home))
  );
  assert!(status(&a).unwrap().changes.is_empty());
}

#[test]
fn a_restore_keeps_the_local_source_path() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    set_source(a, FOO, folder("/a/foo"));
    add_instruction(a, NOTE, "# v1\n");
    set_instruction_source(a, NOTE, file_source("/a/rules.md"));
  });
  // B got the source kinds without the path of A.
  assert_eq!(skill_source(&b, FOO), folder(""));
  assert_eq!(instruction_source(&b, NOTE), file_source(""));

  // B has the same items from its own folders.
  set_source(&b, FOO, folder("/b/foo"));
  set_instruction_source(&b, NOTE, file_source("/b/rules.md"));
  assert!(status(&b).unwrap().changes.is_empty());

  edit_skill(&a, FOO, "foo", "v2");
  fs::write(a.instruction_file(NOTE), "# v2\n").unwrap();
  instruction_service::update_lock(&a, |lock| {
    lock.instructions.get_mut(NOTE).unwrap().hash =
      crate::utils::hash::hash_file(&a.instruction_file(NOTE)).unwrap();
    Ok(())
  })
  .unwrap();
  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);

  pull(&b, &repo);
  assert!(skill_body(&b, FOO, "foo").contains("v2"));
  assert_eq!(skill_source(&b, FOO), folder("/b/foo"));
  assert_eq!(instruction_source(&b, NOTE), file_source("/b/rules.md"));
  // The sources do not change on each sync.
  assert!(status(&b).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&b, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  pull(&a, &repo);
  assert_eq!(skill_source(&a, FOO), folder("/a/foo"));
  assert_eq!(instruction_source(&a, NOTE), file_source("/a/rules.md"));
}

#[test]
fn a_first_merge_does_not_pair_equal_folder_paths() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "from a");
  set_source(&a, FOO, folder("/Users/me/skills/foo"));
  backup_with(&a, &repo).unwrap();

  let b = computer(tmp.path(), "b", &["claude-code"]);
  add_skill(&b, BAR, "foo", "from b");
  set_source(&b, BAR, folder("/Users/me/skills/foo"));
  let planned = plan(&b, &repo, RestoreMode::Merge);
  let foo = item(&planned, &skill_key(FOO));
  assert_eq!(
    (foo.local_id.as_deref(), foo.resolution),
    (None, Resolution::TakeRemote)
  );
  assert_eq!(
    item(&planned, &skill_key(BAR)).resolution,
    Resolution::KeepLocal
  );
}

#[test]
fn source_paths_in_an_existing_repository_go_on_the_next_backup() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, _) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    set_source(a, FOO, folder("/a/foo"));
    add_instruction(a, NOTE, "# rules\n");
    set_instruction_source(a, NOTE, file_source("/a/rules.md"));
  });
  // An earlier version wrote the paths of another computer.
  let mut library = library_at_head(&repo);
  library.skills.get_mut(FOO).unwrap().source = folder("/old/foo");
  library.instructions.get_mut(NOTE).unwrap().source = file_source("/old/rules.md");
  let mut theirs = library.skills[FOO].clone();
  theirs.name = "theirs".to_string();
  theirs.scope = Scope::Device(OTHER_DEVICE.to_string());
  library.skills.insert(OTHER_SKILL.to_string(), theirs);
  commit_files(
    &repo,
    repo.files_at(&repo.head_commit(), ""),
    &[
      (LIBRARY_FILE, pretty_json(&library).unwrap()),
      (
        &format!("skills/{}/theirs/SKILL.md", OTHER_SKILL),
        b"theirs".to_vec(),
      ),
    ],
    "Backup from an old app",
  );

  // A new computer reads the library and gets no path of another computer.
  let c = computer(tmp.path(), "c", &["claude-code"]);
  assert_eq!(pull(&c, &repo).result, RestoreResult::Applied);
  assert_eq!(skill_source(&c, FOO), folder(""));
  assert_eq!(instruction_source(&c, NOTE), file_source(""));

  // A keeps its own paths, and its next backup removes the paths from the repository.
  pull(&a, &repo);
  assert_eq!(skill_source(&a, FOO), folder("/a/foo"));
  assert_eq!(instruction_source(&a, NOTE), file_source("/a/rules.md"));
  backup_with(&a, &repo).unwrap();
  let text = String::from_utf8(repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  assert!(!text.contains("/old/"), "{}", text);
  assert_eq!(library_at_head(&repo).skills[OTHER_SKILL].name, "theirs");
}

/// Replace the head with a commit whose `library.json` holds an absolute path in each local
/// source, as versions before #72 wrote it.
fn put_old_paths(repo: &FakeRepo) {
  let mut library: serde_json::Value =
    serde_json::from_slice(&repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  for kind in ["skills", "instructions"] {
    for (id, record) in library[kind].as_object_mut().unwrap().iter_mut() {
      let source = record["source"].as_object_mut().unwrap();
      if source.contains_key("path") {
        source.insert("path".to_string(), format!("/old/{}", id).into());
      }
    }
  }
  commit_files(
    repo,
    repo.files_at(&repo.head_commit(), ""),
    &[(LIBRARY_FILE, pretty_json(&library).unwrap())],
    "Backup from an old app",
  );
}

fn assert_no_old_paths(repo: &FakeRepo) {
  let text = String::from_utf8(repo.head_file(LIBRARY_FILE).unwrap()).unwrap();
  assert!(!text.contains("/old/"), "{}", text);
}

#[test]
fn an_old_path_of_a_deferred_item_is_not_written_back() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    set_source(a, FOO, folder("/a/foo"));
  });
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();
  put_old_paths(&repo);

  // B leaves the change out, then backs up another item.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  apply(
    &b,
    &repo,
    &planned,
    RestoreDecisions {
      include: BTreeMap::from([(skill_key(FOO), false)]),
      ..Default::default()
    },
  );
  assert!(skill_body(&b, FOO, "foo").contains("v1"));
  add_skill(&b, BAR, "bar", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert_no_old_paths(&repo);
  // The deferred item is still offered, excluded by default, and B has nothing more to back up.
  let planned = plan(&b, &repo, RestoreMode::Pull);
  assert!(item(&planned, &skill_key(FOO)).deferred);
  assert!(status(&b).unwrap().changes.is_empty());
}

#[test]
fn an_old_path_of_an_incoming_item_is_not_written_back() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let (a, b) = synced_pair(tmp.path(), &repo, |a| {
    add_skill(a, FOO, "foo", "v1");
    set_source(a, FOO, folder("/a/foo"));
  });
  edit_skill(&a, FOO, "foo", "v2");
  backup_with(&a, &repo).unwrap();
  put_old_paths(&repo);

  // B backs up another item on the moved head; the change of A becomes incoming.
  add_skill(&b, BAR, "bar", "v1");
  let backup = backup_with(&b, &repo).unwrap();
  assert_eq!((backup.result, backup.incoming), (BackupResult::Pushed, 1));
  assert_no_old_paths(&repo);
  add_skill(&b, BAZ, "baz", "v1");
  assert_eq!(backup_with(&b, &repo).unwrap().result, BackupResult::Pushed);
  assert_no_old_paths(&repo);
  assert!(status(&b).unwrap().changes.is_empty());

  pull(&b, &repo);
  assert!(skill_body(&b, FOO, "foo").contains("v2"));
  assert_eq!(skill_source(&b, FOO), folder(""));
}

#[test]
fn the_migration_removes_old_paths() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  set_source(&a, FOO, folder("/a/foo"));
  add_instruction(&a, NOTE, "# rules\n");
  set_instruction_source(&a, NOTE, file_source("/a/rules.md"));
  backup_with(&a, &repo).unwrap();
  downgrade_head(&repo, &[]);
  put_old_paths(&repo);
  old_app_state(&a, &repo);

  migrate_with(&a, &repo).unwrap();
  assert_eq!(library_at_head(&repo).version, 2);
  assert_no_old_paths(&repo);
  assert!(status(&a).unwrap().changes.is_empty());
  assert_eq!(
    backup_with(&a, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  assert_eq!(skill_source(&a, FOO), folder("/a/foo"));
}

#[test]
fn an_unchanged_backup_removes_old_paths_once() {
  let tmp = tempfile::tempdir().unwrap();
  let repo = FakeRepo::initialized();
  let a = computer(tmp.path(), "a", &["claude-code"]);
  add_skill(&a, FOO, "foo", "v1");
  set_source(&a, FOO, folder("/a/foo"));
  backup_with(&a, &repo).unwrap();
  let clean = repo.head_commit();
  put_old_paths(&repo);
  // A restore commits its merged library, which has no old paths.
  assert!(pull(&a, &repo).pushed);
  assert_no_old_paths(&repo);

  // A computer whose base is the commit with the paths, as a sync with an earlier version
  // left it.
  let old = repo.files_at(&repo.parent_of(&repo.head_commit()).unwrap(), "");
  let library = commit_files(&repo, old, &[], "Backup from an old app");
  assert_ne!(library, clean);
  let mut state = read_state(&a).unwrap();
  state.last_synced_commit = Some(library);
  state.base_files = repo.files_at(&repo.head_commit(), "");
  state.base_library = Some(library_at_head(&repo));
  write_state(&a, &state).unwrap();
  assert!(status(&a).unwrap().changes.is_empty());

  assert_eq!(backup_with(&a, &repo).unwrap().result, BackupResult::Pushed);
  assert_no_old_paths(&repo);
  let commits = repo.commit_count();
  assert_eq!(
    backup_with(&a, &repo).unwrap().result,
    BackupResult::UpToDate
  );
  assert_eq!(repo.commit_count(), commits);
}
