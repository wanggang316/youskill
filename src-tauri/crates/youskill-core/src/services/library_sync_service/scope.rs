//! Item scope changes and copies of the device-only items of other devices.
//!
//! A scope change is a local edit: the sync state keeps it until a backup writes it into
//! `library.json` or `agents.json`. The files of the item do not move. This device can
//! change only the items it gets (the shared items and its own device-only items), so it
//! never changes or removes a device-only item of another device. It can copy one as its
//! own device-only item with a new id (ADR 0003: ids are unique across scopes).

use super::restore::{safe_components, write_entry};
use super::*;
use crate::models::{new_skill_id, InstructionRecord};

/// The scope of an item as this device sees it.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemScope {
  /// Every device gets it.
  Shared,
  /// Only this device gets it.
  Device,
}

/// The scope of a local item; `None` when the library is not connected or the item is not
/// in this library.
pub fn item_scope(env: &Env, key: &str) -> Result<Option<ItemScope>, String> {
  let state = read_state(env)?;
  if state.connected().is_err() {
    return Ok(None);
  }
  let device = state.device();
  let snapshot = local_snapshot(env, &state, &device)?;
  Ok(
    snapshot
      .items()
      .get(key)
      .and_then(|version| version.scope())
      .map(|scope| match scope {
        Scope::Shared => ItemScope::Shared,
        Scope::Device(_) => ItemScope::Device,
      }),
  )
}

/// Make a local item shared or device-only for this device. The next backup writes it.
pub fn set_item_scope(env: &Env, key: &str, scope: ItemScope) -> Result<(), String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  let device = ensure_device(env, &mut state)?;
  if state.deferred.contains_key(key) || state.incoming.contains_key(key) {
    return Err(
      "The repository has another version of this item. Restore it before you change its scope"
        .to_string(),
    );
  }
  let snapshot = local_snapshot(env, &state, &device)?;
  if !snapshot.items().contains_key(key) {
    return Err("This item is not in the library of this computer".to_string());
  }
  let next = match scope {
    ItemScope::Shared => Scope::Shared,
    ItemScope::Device => Scope::Device(device.clone()),
  };
  let base = state
    .base()
    .scope_of(key)
    .filter(|scope| device_gets(scope, &device))
    .cloned()
    .unwrap_or_default();
  if next == base {
    state.scopes.remove(key);
  } else {
    state.scopes.insert(key.to_string(), next);
  }
  write_state(env, &state)
}

/// Copy the device-only item `key` of another device from the branch head into this
/// library as a device-only item of this device, with a new id. Returns the new item key.
pub fn copy_device_item(env: &Env, key: &str) -> Result<String, String> {
  let remote = github_remote(env)?;
  copy_device_item_with(env, &remote, key)
}

pub fn copy_device_item_with(
  env: &Env,
  remote: &dyn RemoteRepo,
  key: &str,
) -> Result<String, String> {
  normalize_library(env)?;
  let _ops = ops_guard(env)?;
  let mut state = read_state(env)?;
  state.connected()?;
  let device = ensure_device(env, &mut state)?;
  let head = remote
    .head()?
    .ok_or_else(|| "The repository holds no library yet".to_string())?;
  let files = remote.tree(&head.tree)?;
  let head_state = load_repo_state(remote, &head.commit, &files, &device)?
    .ok_or_else(|| "The repository holds no library yet".to_string())?;
  let owner = match head_state.scope_of(key) {
    Some(Scope::Device(owner)) if !device_gets(&Scope::Device(owner.clone()), &device) => {
      owner.clone()
    },
    _ => return Err("This item is not a device-only item of another device".to_string()),
  };
  let Some((kind, id)) = split_key(key) else {
    return Err(format!("Unknown item {}", key));
  };
  let version = head_state
    .items(&owner)
    .remove(key)
    .ok_or_else(|| format!("The repository has no item {}", key))?;
  let paths = version.paths(id);
  let contents = if paths.is_empty() {
    BTreeMap::new()
  } else {
    remote.download(&head.commit, &paths)?
  };

  let new_key = match (kind, &version.entry) {
    (ItemKind::Skill, Entry::Skill(skill)) => {
      validate_skill_name(&skill.name)?;
      let new_id = new_skill_id();
      let dir = env.hub_dir(&new_id, &skill.name);
      let prefix = format!("{}/{}/{}/", SKILLS_DIR, id, skill.name);
      let written = (|| {
        for (path, content) in &contents {
          let relative = path
            .strip_prefix(&prefix)
            .and_then(safe_components)
            .ok_or_else(|| format!("The repository has an unsafe path: {}", path))?;
          let mut target = dir.clone();
          for part in relative {
            target = target.join(part);
          }
          write_entry(&target, &paths[path].mode, content)?;
        }
        if !dir.is_dir() {
          return Err(format!("The repository has no files for {}", skill.name));
        }
        invalidate_hash_cache(&dir);
        hash_dir(&dir)
      })();
      let hash = match written {
        Ok(hash) => hash,
        Err(err) => {
          let _ = remove_path_any(&env.hub_root.join(&new_id));
          return Err(err);
        },
      };
      store(env).update(|lock| {
        lock.skills.insert(
          new_id.clone(),
          SkillRecord {
            name: skill.name.clone(),
            source: skill.source.clone(),
            hash,
            imported_at: skill.imported_at.clone(),
            updated_at: now_rfc3339(),
            installs: Vec::new(),
          },
        );
        Ok(())
      })?;
      skill_key(&new_id)
    },
    (ItemKind::Instruction, Entry::Instruction(instruction)) => {
      let content = contents
        .values()
        .next()
        .ok_or_else(|| format!("The repository has no file for {}", instruction.name))?;
      let new_id = uuid::Uuid::new_v4().to_string();
      write_atomic(&env.instruction_file(&new_id), content)?;
      instruction_service::update_lock(env, |lock| {
        lock.instructions.insert(
          new_id.clone(),
          InstructionRecord {
            name: instruction.name.clone(),
            source: instruction.source.clone(),
            hash: instruction.hash.clone(),
            imported_at: instruction.imported_at.clone(),
            updated_at: now_rfc3339(),
            installs: Vec::new(),
          },
        );
        Ok(())
      })?;
      instruction_key(&new_id)
    },
    (ItemKind::Agent, Entry::Agent(agent)) => {
      let mut apps = read_user_agent_apps(env)?;
      let new_id = new_agent_id(&agent.app.id);
      apps.push(AgentApp {
        id: new_id.clone(),
        ..agent.app.clone()
      });
      apps.sort_by(|a, b| a.id.cmp(&b.id));
      write_atomic(&user_agent_apps_path(env), &pretty_json(&apps)?)?;
      agent_key(&new_id)
    },
    _ => return Err(format!("Unknown item {}", key)),
  };
  state
    .scopes
    .insert(new_key.clone(), Scope::Device(device.clone()));
  write_state(env, &state)?;
  Ok(new_key)
}

/// A new id for a copy of the custom agent `id`.
pub(super) fn new_agent_id(id: &str) -> String {
  let suffix = uuid::Uuid::new_v4().simple().to_string();
  format!("{}-{}", id, &suffix[..8])
}

/// Give the local custom agent `from` the id `to`, also in the install records that name it.
pub(super) fn reid_agent(env: &Env, from: &str, to: &str) -> Result<(), String> {
  let mut apps = read_user_agent_apps(env)?;
  for app in apps.iter_mut().filter(|app| app.id == from) {
    app.id = to.to_string();
  }
  apps.sort_by(|a, b| a.id.cmp(&b.id));
  write_atomic(&user_agent_apps_path(env), &pretty_json(&apps)?)?;
  let rename = |installs: &mut Vec<InstallRecord>| {
    for id in installs
      .iter_mut()
      .flat_map(|install| install.agent_ids.iter_mut())
      .filter(|id| id.as_str() == from)
    {
      *id = to.to_string();
    }
  };
  store(env).update(|lock| {
    for record in lock.skills.values_mut() {
      rename(&mut record.installs);
    }
    Ok(())
  })?;
  instruction_service::update_lock(env, |lock| {
    for record in lock.instructions.values_mut() {
      rename(&mut record.installs);
    }
    Ok(())
  })
}
