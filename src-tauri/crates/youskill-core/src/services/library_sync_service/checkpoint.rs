//! The local checkpoint taken before each restore, and the undo that puts it back.
//!
//! A checkpoint holds copies of the lock files, the sync state, the custom agents file and
//! every hub directory, template file or install target the restore replaces, removes,
//! rekeys or creates, plus the symlink installs that point into them. One checkpoint is
//! kept; a restore replaces it only once it is about to write. Undo only changes this
//! computer, never the repository, and refuses when something it would put back changed
//! after the restore.

use super::*;
use crate::services::install_service::write_target;
use crate::utils::folder::{copy_dir, CopyOpts};
use crate::utils::hash::hash_file;
use crate::utils::path::is_symlink;

const CHECKPOINT_DIR: &str = ".sync-checkpoint";
/// Where a new checkpoint is assembled, so a failed one leaves the last one in place.
const NEXT_DIR: &str = ".sync-checkpoint-next";
const MANIFEST_FILE: &str = "manifest.json";
const FILES_DIR: &str = "files";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointInfo {
  pub created_at: String,
  /// The commit the restore applied.
  pub target: String,
}

/// A symlink install to point at `source` again after an undo.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct Link {
  pub target: String,
  pub source: String,
  pub kind: ItemKind,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
  path: String,
  /// Name of the copy under `files/`; `None` when the path did not exist.
  #[serde(default)]
  copy: Option<String>,
  /// Fingerprint when the checkpoint was taken.
  #[serde(default)]
  before: Option<String>,
  /// Fingerprint when the restore finished.
  #[serde(default)]
  after: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
  #[serde(flatten)]
  info: CheckpointInfo,
  saved: Vec<Saved>,
  links: Vec<Link>,
  /// The restore finished and `after` is recorded; undo then refuses later changes.
  #[serde(default)]
  sealed: bool,
}

fn checkpoint_dir(env: &Env) -> PathBuf {
  env.youskill_root.join(CHECKPOINT_DIR)
}

fn read_manifest(env: &Env) -> Option<Manifest> {
  let content = fs::read(checkpoint_dir(env).join(MANIFEST_FILE)).ok()?;
  serde_json::from_slice(&content).ok()
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
  write_atomic(
    &dir.join(MANIFEST_FILE),
    &serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?,
  )
}

pub(super) fn info(env: &Env) -> Option<CheckpointInfo> {
  read_manifest(env).map(|manifest| manifest.info)
}

pub(super) fn discard(env: &Env) {
  for dir in [checkpoint_dir(env), env.youskill_root.join(NEXT_DIR)] {
    if dir.exists() {
      let _ = remove_path_any(&dir);
    }
  }
}

fn copy_entry(from: &Path, to: &Path) -> Result<(), String> {
  if let Some(parent) = to.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  if from.is_dir() && !is_symlink(from) {
    copy_dir(from, to, &CopyOpts::all())
  } else {
    fs::copy(from, to).map(|_| ()).map_err(|e| e.to_string())
  }
}

/// What a path holds, to tell whether it changed. The skill lock leaves out the source
/// check fields, which a background update check rewrites; the sync state is not compared.
fn fingerprint(env: &Env, path: &Path) -> Option<String> {
  if path == state_path(env) {
    return None;
  }
  if is_symlink(path) {
    return fs::read_link(path)
      .ok()
      .map(|target| format!("link:{}", target.to_string_lossy()));
  }
  if !path.exists() {
    return None;
  }
  if path == env.lock_path {
    let mut lock = store(env).read().ok()?;
    for record in lock.skills.values_mut() {
      record.source = shared_source(&record.source);
    }
    return serde_json::to_vec(&lock.skills)
      .ok()
      .map(|bytes| blob_sha(&bytes));
  }
  if path.is_dir() {
    hash_dir(path).ok()
  } else {
    hash_file(path).ok()
  }
}

fn save(dir: &Path, env: &Env, path: &Path, index: usize) -> Result<Saved, String> {
  let copy = if path.exists() || is_symlink(path) {
    let name = index.to_string();
    copy_entry(path, &dir.join(FILES_DIR).join(&name))?;
    Some(name)
  } else {
    None
  };
  Ok(Saved {
    path: path.to_string_lossy().to_string(),
    copy,
    before: fingerprint(env, path),
    after: None,
  })
}

/// Save copies of `paths` as the new checkpoint. It is assembled next to the current one
/// and replaces it only when complete, so a failure here keeps the last checkpoint.
pub(super) fn take(
  env: &Env,
  paths: &[PathBuf],
  links: &[Link],
  target: &str,
) -> Result<(), String> {
  let next = env.youskill_root.join(NEXT_DIR);
  if next.exists() {
    remove_path_any(&next)?;
  }
  fs::create_dir_all(next.join(FILES_DIR)).map_err(|e| e.to_string())?;
  let saved = paths
    .iter()
    .enumerate()
    .map(|(index, path)| save(&next, env, path, index))
    .collect::<Result<Vec<_>, _>>();
  let saved = match saved {
    Ok(saved) => saved,
    Err(err) => {
      let _ = remove_path_any(&next);
      return Err(err);
    },
  };
  write_manifest(
    &next,
    &Manifest {
      info: CheckpointInfo {
        created_at: now_rfc3339(),
        target: target.to_string(),
      },
      saved,
      links: links.to_vec(),
      sealed: false,
    },
  )?;
  let dir = checkpoint_dir(env);
  if dir.exists() {
    remove_path_any(&dir)?;
  }
  fs::rename(&next, &dir).map_err(|e| e.to_string())
}

/// Add `paths` that are not saved yet to the current checkpoint, before something more
/// (the installs of a restore) changes them.
pub(super) fn add(env: &Env, paths: &[PathBuf]) -> Result<(), String> {
  let dir = checkpoint_dir(env);
  let mut manifest = read_manifest(env).ok_or_else(|| "There is no checkpoint".to_string())?;
  for path in paths {
    let text = path.to_string_lossy();
    if manifest.saved.iter().any(|saved| saved.path == text) {
      continue;
    }
    let index = manifest.saved.len();
    manifest.saved.push(save(&dir, env, path, index)?);
  }
  write_manifest(&dir, &manifest)
}

/// Record what the restore left, so an undo can tell later changes apart.
pub(super) fn seal(env: &Env) -> Result<(), String> {
  let dir = checkpoint_dir(env);
  let mut manifest = read_manifest(env).ok_or_else(|| "There is no checkpoint".to_string())?;
  for saved in &mut manifest.saved {
    saved.after = fingerprint(env, Path::new(&saved.path));
  }
  manifest.sealed = true;
  write_manifest(&dir, &manifest)
}

/// Put the last checkpoint back on this computer: lock files, sync state, custom agents,
/// install targets and the hub contents and ids the restore changed. The repository is not
/// touched; the next backup or restore compares against the restored base. Refused when a
/// saved path changed after the restore, since undo would discard that change. A path
/// already put back by an earlier, interrupted undo counts as unchanged, so undo can be
/// retried; the checkpoint stays until every step succeeds.
pub fn undo_restore(env: &Env) -> Result<(), String> {
  let _ops = ops_guard(env)?;
  let dir = checkpoint_dir(env);
  let manifest = read_manifest(env).ok_or_else(|| "There is no restore to undo".to_string())?;
  if manifest.sealed {
    let changed: Vec<&str> = manifest
      .saved
      .iter()
      .filter(|saved| {
        let now = fingerprint(env, Path::new(&saved.path));
        now != saved.after && now != saved.before
      })
      .map(|saved| saved.path.as_str())
      .collect();
    if !changed.is_empty() {
      return Err(format!(
        "Changed after the restore, so undo would discard these changes: {}",
        changed.join(", ")
      ));
    }
  }

  for saved in &manifest.saved {
    let path = Path::new(&saved.path);
    if path.exists() || is_symlink(path) {
      remove_path_any(path)?;
    }
    if let Some(copy) = &saved.copy {
      copy_entry(&dir.join(FILES_DIR).join(copy), path)?;
    }
    invalidate_hash_cache(path);
    // `skills/<id>/` holds one skill directory; its cached hash is keyed by that one.
    if let Ok(entries) = fs::read_dir(path) {
      for entry in entries.flatten() {
        invalidate_hash_cache(&entry.path());
      }
    }
  }
  let mut errors = Vec::new();
  for link in &manifest.links {
    let (source, target) = (Path::new(&link.source), Path::new(&link.target));
    let result = match link.kind {
      ItemKind::Instruction => {
        instruction_service::write_target(source, target, InstallMode::Symlink).map(|_| ())
      },
      _ => write_target(source, target, InstallMode::Symlink).map(|_| ()),
    };
    if let Err(err) = result {
      errors.push(format!("{}: {}", link.target, err));
    }
  }
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  discard(env);
  Ok(())
}
