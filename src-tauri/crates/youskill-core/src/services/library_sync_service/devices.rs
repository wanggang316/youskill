//! The devices of the repository: list, rename, take over, copy installs and remove.
//!
//! The device name is for display only and is stored in the device file. Take over and
//! copy installs are choices of the first connect: a device that synced before keeps its
//! own id. Device removal writes one commit that sets the scope or the owner of each
//! device-only item of the device (or removes it) and deletes its device file.

use super::*;

/// One device of the repository.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
  pub id: String,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_backup_at: Option<String>,
  /// User-level installs in its device file.
  pub install_count: usize,
  /// This computer.
  pub this_device: bool,
  /// The repository has a device file for it.
  pub has_file: bool,
  /// Its device-only items.
  pub items: Vec<DeviceItem>,
}

/// A device-only item of a device.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceItem {
  /// `skill:<id>`, `instruction:<id>` or `agent:<id>`.
  pub key: String,
  pub kind: ItemKind,
  pub id: String,
  pub name: String,
}

/// What happens to a device-only item of a removed device.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DeviceItemChoice {
  /// Every device gets it.
  Share,
  /// It becomes a device-only item of `device`.
  Give {
    device: String,
  },
  Delete,
}

/// The branch head with its library, custom agents and every device file.
struct Head {
  commit: String,
  files: BTreeMap<String, TreeFile>,
  library: LibraryFile,
  agents: Vec<SyncedAgent>,
  devices: BTreeMap<String, DeviceFile>,
}

fn read_head(remote: &dyn RemoteRepo) -> Result<Option<Head>, String> {
  let Some(head) = remote.head()? else {
    return Ok(None);
  };
  let files = remote.tree(&head.tree)?;
  let Some(state) = load_repo_state(remote, &head.commit, &files, "")? else {
    return Ok(None);
  };
  if state.library.version < LIBRARY_VERSION {
    return Err("Update the repository to the new library format first".to_string());
  }
  let prefix = format!("{}/", DEVICES_DIR);
  let wanted: BTreeMap<String, TreeFile> = files
    .iter()
    .filter(|(path, _)| path.starts_with(&prefix) && path.ends_with(".json"))
    .map(|(path, file)| (path.clone(), file.clone()))
    .collect();
  let contents = if wanted.is_empty() {
    BTreeMap::new()
  } else {
    remote.download(&head.commit, &wanted)?
  };
  let mut devices = BTreeMap::new();
  for (path, content) in contents {
    let id = path[prefix.len()..path.len() - ".json".len()].to_string();
    let file: DeviceFile = serde_json::from_slice(&content)
      .map_err(|e| format!("The repository's {} is not valid: {}", path, e))?;
    devices.insert(id, file);
  }
  Ok(Some(Head {
    commit: head.commit,
    files,
    library: state.library,
    agents: state.agents,
    devices,
  }))
}

impl Head {
  /// Device-only items by owner device id.
  fn device_items(&self) -> BTreeMap<String, Vec<DeviceItem>> {
    let mut owners: BTreeMap<String, Vec<DeviceItem>> = BTreeMap::new();
    let mut add = |scope: &Scope, key: String, kind: ItemKind, id: &str, name: &str| {
      if let Scope::Device(owner) = scope {
        owners.entry(owner.clone()).or_default().push(DeviceItem {
          key,
          kind,
          id: id.to_string(),
          name: name.to_string(),
        });
      }
    };
    for (id, skill) in &self.library.skills {
      add(
        &skill.scope,
        skill_key(id),
        ItemKind::Skill,
        id,
        &skill.name,
      );
    }
    for (id, instruction) in &self.library.instructions {
      add(
        &instruction.scope,
        instruction_key(id),
        ItemKind::Instruction,
        id,
        &instruction.name,
      );
    }
    for agent in &self.agents {
      add(
        &agent.scope,
        agent_key(&agent.app.id),
        ItemKind::Agent,
        &agent.app.id,
        &agent.app.display_name,
      );
    }
    for items in owners.values_mut() {
      items.sort_by_key(|item| (item.kind, item.name.to_lowercase()));
    }
    owners
  }
}

/// A device that has never synced: it may still take over a device or copy its installs.
fn first_connect(state: &SyncState) -> Result<(), String> {
  if state.last_synced_commit.is_some() {
    return Err(
      "Only a computer that has not synced with this repository yet can take over a device or copy its installs"
        .to_string(),
    );
  }
  Ok(())
}

pub fn devices(env: &Env) -> Result<Vec<DeviceInfo>, String> {
  let remote = github_remote(env)?;
  devices_with(env, &remote)
}

/// The devices of the repository: those with a device file, owners of device-only items,
/// and this computer. This computer comes first, then by name.
pub fn devices_with(env: &Env, remote: &dyn RemoteRepo) -> Result<Vec<DeviceInfo>, String> {
  let state = read_state(env)?;
  state.connected()?;
  let this = state.device();
  let Some(head) = read_head(remote)? else {
    return Ok(Vec::new());
  };
  let mut owners = head.device_items();
  let ids: BTreeSet<String> = head
    .devices
    .keys()
    .chain(owners.keys())
    .cloned()
    .chain((!this.is_empty()).then(|| this.clone()))
    .collect();
  let mut list: Vec<DeviceInfo> = ids
    .into_iter()
    .map(|id| {
      let file = head.devices.get(&id);
      let this_device = id == this;
      DeviceInfo {
        name: match file {
          _ if this_device => state.device_label(),
          Some(file) => file.name.clone(),
          None => String::new(),
        },
        last_backup_at: file.and_then(|file| file.last_backup_at.clone()),
        install_count: file
          .map(|file| file.installs.values().map(Vec::len).sum())
          .unwrap_or(0),
        this_device,
        has_file: file.is_some(),
        items: owners.remove(&id).unwrap_or_default(),
        id,
      }
    })
    .collect();
  list.sort_by(|a, b| {
    (!a.this_device, a.name.to_lowercase(), &a.id).cmp(&(
      !b.this_device,
      b.name.to_lowercase(),
      &b.id,
    ))
  });
  Ok(list)
}

/// Change the display name of this device. The next backup writes it into the device file.
pub fn rename_device(env: &Env, name: &str) -> Result<SyncStatus, String> {
  let name = name.trim();
  if name.is_empty() {
    return Err("Enter a device name".to_string());
  }
  {
    let _ops = ops_guard(env)?;
    let mut state = read_state(env)?;
    state.connected()?;
    state.device_name = Some(name.to_string());
    write_state(env, &state)?;
  }
  status(env)
}

pub fn take_over_device(env: &Env, device: &str) -> Result<SyncStatus, String> {
  let remote = github_remote(env)?;
  take_over_device_with(env, &remote, device)
}

/// Make this computer the device `device` on its first connect: it gets the id, and the
/// following restore brings the device-only items and the installs of that device file
/// (as pending installs). The old computer must no longer sync: two computers with one id
/// write one device file.
pub fn take_over_device_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  device: &str,
) -> Result<SyncStatus, String> {
  {
    let _ops = ops_guard(env)?;
    let mut state = read_state(env)?;
    state.connected()?;
    first_connect(&state)?;
    if state.device_id.as_deref() == Some(device) {
      return Err("This computer is that device already".to_string());
    }
    let head =
      read_head(remote)?.ok_or_else(|| "The repository holds no library yet".to_string())?;
    let file = head
      .devices
      .get(device)
      .ok_or_else(|| format!("The repository has no device {}", device))?;
    state.device_id = Some(device.to_string());
    state.device_name = Some(file.name.clone());
    state.base_device = None;
    state.scopes.clear();
    write_state(env, &state)?;
  }
  status(env)
}

pub fn copy_device_installs(env: &Env, from: Option<&str>) -> Result<SyncStatus, String> {
  let remote = github_remote(env)?;
  copy_device_installs_with(env, &remote, from)
}

/// Start this device from the installs of another device on its first connect, for the
/// agents of this computer only. `None` copies the device with the latest backup. The
/// following restore shows them as pending installs; the source device file does not
/// change.
pub fn copy_device_installs_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  from: Option<&str>,
) -> Result<SyncStatus, String> {
  {
    let _ops = ops_guard(env)?;
    let mut state = read_state(env)?;
    state.connected()?;
    first_connect(&state)?;
    let this = ensure_device(env, &mut state)?;
    let head =
      read_head(remote)?.ok_or_else(|| "The repository holds no library yet".to_string())?;
    let source = match from {
      Some(id) => head
        .devices
        .get(id)
        .filter(|_| id != this)
        .ok_or_else(|| format!("The repository has no other device {}", id))?,
      None => head
        .devices
        .iter()
        .filter(|(id, _)| **id != this)
        .max_by(|(_, a), (_, b)| a.last_backup_at.cmp(&b.last_backup_at))
        .map(|(_, file)| file)
        .ok_or_else(|| "The repository has no other device".to_string())?,
    };
    let installs = source
      .installs
      .iter()
      .filter(|(agent, _)| env.agent(agent).is_some())
      .map(|(agent, list)| (agent.clone(), list.clone()))
      .collect();
    // The device file this device starts from: the first restore keeps it as the base of
    // the installs here, so they show as pending installs.
    state.base_device = Some(DeviceFile {
      name: state.device_label(),
      installs,
      last_backup_at: None,
    });
    write_state(env, &state)?;
  }
  status(env)
}

pub fn remove_device(
  env: &Env,
  device: &str,
  choices: &BTreeMap<String, DeviceItemChoice>,
) -> Result<(), String> {
  let remote = github_remote(env)?;
  remove_device_with(env, &remote, device, choices)
}

/// Remove the device `device` from the repository in one commit: each of its device-only
/// items becomes shared, goes to another device or is deleted as `choices` says (one
/// choice for each item), then its device file is deleted. Other device files do not
/// change. This computer cannot remove itself; it disconnects instead.
pub fn remove_device_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  device: &str,
  choices: &BTreeMap<String, DeviceItemChoice>,
) -> Result<(), String> {
  let _ops = ops_guard(env)?;
  let state = read_state(env)?;
  state.connected()?;
  let this = state.device();
  if device == this {
    return Err("This computer cannot remove itself. Disconnect instead".to_string());
  }
  let head = read_head(remote)?.ok_or_else(|| "The repository holds no library yet".to_string())?;
  let owned = head.device_items().remove(device).unwrap_or_default();
  let file_path = device_path(device);
  if owned.is_empty() && !head.files.contains_key(&file_path) {
    return Err(format!("The repository has no device {}", device));
  }
  for key in choices.keys() {
    if !owned.iter().any(|item| &item.key == key) {
      return Err(format!("{} is not a device-only item of that device", key));
    }
  }
  let receivers: BTreeSet<&str> = head
    .devices
    .keys()
    .map(String::as_str)
    .chain((!this.is_empty()).then_some(this.as_str()))
    .filter(|id| *id != device)
    .collect();

  let mut library = head.library.clone();
  let mut agents = head.agents.clone();
  let mut files = head.files.clone();
  files.remove(&file_path);
  for item in &owned {
    let choice = choices.get(&item.key).ok_or_else(|| {
      format!(
        "Choose what happens to {}",
        if item.name.is_empty() {
          &item.key
        } else {
          &item.name
        }
      )
    })?;
    let scope = match choice {
      DeviceItemChoice::Share => Some(Scope::Shared),
      DeviceItemChoice::Give { device: to } if receivers.contains(to.as_str()) => {
        Some(Scope::Device(to.clone()))
      },
      DeviceItemChoice::Give { device: to } => {
        return Err(format!("The repository has no other device {}", to))
      },
      DeviceItemChoice::Delete => None,
    };
    let id = item.id.as_str();
    match (item.kind, scope) {
      (ItemKind::Skill, Some(scope)) => library.skills.get_mut(id).unwrap().scope = scope,
      (ItemKind::Instruction, Some(scope)) => {
        library.instructions.get_mut(id).unwrap().scope = scope
      },
      (ItemKind::Agent, Some(scope)) => {
        for agent in agents.iter_mut().filter(|agent| agent.app.id == id) {
          agent.scope = scope.clone();
        }
      },
      (ItemKind::Skill, None) => {
        library.skills.remove(id);
        let prefix = format!("{}/{}/", SKILLS_DIR, id);
        files.retain(|path, _| !path.starts_with(&prefix));
      },
      (ItemKind::Instruction, None) => {
        library.instructions.remove(id);
        files.remove(&format!("{}/{}.md", INSTRUCTIONS_DIR, id));
      },
      (ItemKind::Agent, None) => agents.retain(|agent| agent.app.id != id),
      (ItemKind::Device, _) => {},
    }
  }

  for (path, content) in [
    (LIBRARY_FILE, pretty_json(&library)?),
    (AGENTS_FILE, pretty_json(&agents)?),
    (README_FILE, readme(&library)),
  ] {
    let sha = remote.create_blob(&content)?;
    files.insert(
      path.to_string(),
      TreeFile {
        mode: MODE_FILE.to_string(),
        sha,
      },
    );
  }
  let name = head
    .devices
    .get(device)
    .map(|file| file.name.clone())
    .filter(|name| !name.is_empty())
    .unwrap_or_else(|| device.to_string());
  let tree = remote.create_tree(&files)?;
  let commit = remote.create_commit(
    &format!("Remove device {} on {}", name, state.device_label()),
    &tree,
    Some(&head.commit),
  )?;
  match remote.update_head(&commit) {
    Ok(()) => Ok(()),
    Err(UpdateError::Rejected) => {
      Err("The repository changed while the device was removed. Try again".to_string())
    },
    Err(UpdateError::Other(err)) => Err(err),
  }
}
