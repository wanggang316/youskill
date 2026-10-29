//! The local checkpoint taken before each restore, and the undo that puts it back.
//!
//! A checkpoint holds copies of the lock files, the sync state, the custom agents file and
//! every hub directory or template file the restore replaces, removes or rekeys, plus the
//! symlink installs that point into them. One checkpoint is kept; the next restore replaces
//! it. Undo only changes this computer, never the repository.

use super::*;
use crate::services::install_service::write_target;
use crate::utils::folder::{copy_dir, CopyOpts};
use crate::utils::path::is_symlink;

const CHECKPOINT_DIR: &str = ".sync-checkpoint";
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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
  #[serde(flatten)]
  info: CheckpointInfo,
  saved: Vec<Saved>,
  links: Vec<Link>,
}

fn checkpoint_dir(env: &Env) -> PathBuf {
  env.youskill_root.join(CHECKPOINT_DIR)
}

fn read_manifest(env: &Env) -> Option<Manifest> {
  let content = fs::read(checkpoint_dir(env).join(MANIFEST_FILE)).ok()?;
  serde_json::from_slice(&content).ok()
}

pub(super) fn info(env: &Env) -> Option<CheckpointInfo> {
  read_manifest(env).map(|manifest| manifest.info)
}

pub(super) fn discard(env: &Env) {
  let dir = checkpoint_dir(env);
  if dir.exists() {
    let _ = remove_path_any(&dir);
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

/// Replace the checkpoint with copies of `paths`. The manifest is written last, so an
/// interrupted checkpoint is not offered for undo.
pub(super) fn take(
  env: &Env,
  paths: &[PathBuf],
  links: &[Link],
  target: &str,
) -> Result<(), String> {
  discard(env);
  let dir = checkpoint_dir(env);
  fs::create_dir_all(dir.join(FILES_DIR)).map_err(|e| e.to_string())?;
  let mut saved = Vec::new();
  for (index, path) in paths.iter().enumerate() {
    let copy = if path.exists() {
      let name = index.to_string();
      copy_entry(path, &dir.join(FILES_DIR).join(&name))?;
      Some(name)
    } else {
      None
    };
    saved.push(Saved {
      path: path.to_string_lossy().to_string(),
      copy,
    });
  }
  let manifest = Manifest {
    info: CheckpointInfo {
      created_at: now_rfc3339(),
      target: target.to_string(),
    },
    saved,
    links: links.to_vec(),
  };
  write_atomic(
    &dir.join(MANIFEST_FILE),
    &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
  )
}

/// Put the last checkpoint back on this computer: lock files, sync state, custom agents and
/// the hub contents and ids the restore changed. The repository is not touched; the next
/// backup or restore compares against the restored base.
pub fn undo_restore(env: &Env) -> Result<(), String> {
  let _ops = ops_guard(env)?;
  let dir = checkpoint_dir(env);
  let manifest = read_manifest(env).ok_or_else(|| "There is no restore to undo".to_string())?;
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
  discard(env);
  if !errors.is_empty() {
    return Err(errors.join("\n"));
  }
  Ok(())
}
