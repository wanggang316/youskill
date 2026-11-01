//! Migrate a version 1 repository to version 2 (item scopes and device files).
//!
//! The first device that syncs with version 2 writes one commit after the user confirms:
//! `library.json` version 2 with every item shared and no `agents`, `agents.json` with
//! shared custom agents, and its own device file from its local install records (not from
//! the old `agents` values, which are a union over all devices). Other devices read a
//! version 1 base as version 2 with every item shared and no installs, and add their own
//! device file on their next backup.

use super::*;

pub fn migrate(env: &Env) -> Result<(), String> {
  let remote = github_remote(env)?;
  migrate_with(env, &remote)
}

/// Write the migration commit when the branch head holds a version 1 library. Nothing
/// happens for a version 2 library or an empty repository. When the head is this
/// computer's base, the migration commit becomes the base.
pub fn migrate_with(env: &Env, remote: &dyn RemoteRepo) -> Result<(), String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  let device = ensure_device(env, &mut state)?;
  let Some(head) = remote.head()? else {
    return Ok(());
  };
  let files = remote.tree(&head.tree)?;
  let Some(current) = load_repo_state(remote, &head.commit, &files, &device)? else {
    return Ok(());
  };
  if current.library.version >= LIBRARY_VERSION {
    return Ok(());
  }

  let mut library = current.library.clone();
  library.version = LIBRARY_VERSION;
  for skill in library.skills.values_mut() {
    skill.scope = Scope::Shared;
  }
  for instruction in library.instructions.values_mut() {
    instruction.scope = Scope::Shared;
  }
  let agents: Vec<SyncedAgent> = current
    .agents
    .iter()
    .map(|agent| SyncedAgent {
      scope: Scope::Shared,
      ..agent.clone()
    })
    .collect();

  // Installs of the repository's items on this computer.
  let mut local_agents = BTreeMap::new();
  for (id, record) in &store(env).read()?.skills {
    let agents = user_agents(&record.installs);
    if library.skills.contains_key(id) && !agents.is_empty() {
      local_agents.insert(skill_key(id), agents);
    }
  }
  for (id, record) in &instruction_service::read_lock(env)?.instructions {
    let agents = user_agents(&record.installs);
    if library.instructions.contains_key(id) && !agents.is_empty() {
      local_agents.insert(instruction_key(id), agents);
    }
  }
  let device_file = DeviceFile {
    name: state.device_label(),
    installs: DeviceFile::installs_from(&local_agents),
    last_backup_at: Some(now_rfc3339()),
  };

  let file = |content: Vec<u8>| (MODE_FILE.to_string(), content);
  let contents = BTreeMap::from([
    (LIBRARY_FILE.to_string(), file(pretty_json(&library)?)),
    (AGENTS_FILE.to_string(), file(pretty_json(&agents)?)),
    (README_FILE.to_string(), file(readme(&library))),
    (device_path(&device), file(pretty_json(&device_file)?)),
  ]);
  let mut tree = files.clone();
  for (path, (mode, content)) in &contents {
    tree.insert(
      path.clone(),
      TreeFile {
        mode: mode.clone(),
        sha: blob_sha(content),
      },
    );
  }
  upload_blobs(remote, &contents, &tree, &[&files])?;
  let tree_sha = remote.create_tree(&tree)?;
  let commit = remote.create_commit(
    &format!(
      "Migrate the library to version {} on {}",
      LIBRARY_VERSION,
      state.device_label()
    ),
    &tree_sha,
    Some(&head.commit),
  )?;
  match remote.update_head(&commit) {
    Ok(()) => {},
    Err(UpdateError::Rejected) => {
      return Err("The repository changed during the migration. Try again".to_string())
    },
    Err(UpdateError::Other(err)) => return Err(err),
  }

  if state.last_synced_commit.as_deref() == Some(head.commit.as_str()) {
    state.last_synced_commit = Some(commit);
    state.base_files = tree;
    state.base_library = Some(library);
    state.base_agents = Some(agents);
    state.base_device = Some(device_file);
    state.base_local_agents = local_agents;
    write_state(env, &state)?;
  }
  Ok(())
}
