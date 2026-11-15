use crate::config::load_config;
use crate::models::{
  AgentFileView, DetectedInstruction, InstallMode, InstallRequest, InstallScope,
  InstructionActionResult, InstructionImportItem, InstructionImportOutcome, InstructionView,
  SkillDiff, SyncAction, UninstallRequest,
};
use crate::services::env::Env;
use crate::services::instruction_service;

async fn blocking<T, F>(label: &'static str, f: F) -> Result<T, String>
where
  T: Send + 'static,
  F: FnOnce() -> Result<T, String> + Send + 'static,
{
  tauri::async_runtime::spawn_blocking(f)
    .await
    .map_err(|e| format!("{} join error: {}", label, e))?
}

#[tauri::command]
pub async fn list_instructions() -> Result<Vec<InstructionView>, String> {
  let env = Env::current()?;
  blocking("list_instructions", move || {
    instruction_service::list_instructions(&env)
  })
  .await
}

#[tauri::command]
pub async fn get_instruction(id: String) -> Result<Option<InstructionView>, String> {
  let env = Env::current()?;
  blocking("get_instruction", move || {
    instruction_service::get_instruction(&env, &id)
  })
  .await
}

#[tauri::command]
pub async fn read_instruction(id: String) -> Result<String, String> {
  let env = Env::current()?;
  blocking("read_instruction", move || {
    instruction_service::read_instruction(&env, &id)
  })
  .await
}

#[tauri::command]
pub async fn import_instructions(
  items: Vec<InstructionImportItem>,
) -> Result<Vec<InstructionImportOutcome>, String> {
  let env = Env::current()?;
  blocking("import_instructions", move || {
    instruction_service::import_instructions(&env, items)
  })
  .await
}

#[tauri::command]
pub async fn rename_instruction(id: String, name: String) -> Result<InstructionView, String> {
  let env = Env::current()?;
  blocking("rename_instruction", move || {
    instruction_service::rename_instruction(&env, &id, &name)
  })
  .await
}

#[tauri::command]
pub async fn list_instruction_files() -> Result<Vec<AgentFileView>, String> {
  let env = Env::current()?;
  blocking("list_instruction_files", move || {
    instruction_service::list_instruction_files(&env)
  })
  .await
}

#[tauri::command]
pub async fn list_scope_instruction_files(
  scope: InstallScope,
  project_path: Option<String>,
) -> Result<Vec<AgentFileView>, String> {
  let env = Env::current()?;
  blocking("list_scope_instruction_files", move || {
    instruction_service::list_scope_instruction_files(&env, scope, project_path.as_deref())
  })
  .await
}

#[tauri::command]
pub async fn read_instruction_file(path: String) -> Result<String, String> {
  let env = Env::current()?;
  blocking("read_instruction_file", move || {
    instruction_service::read_instruction_file(&env, &path)
  })
  .await
}

#[tauri::command]
pub async fn write_instruction_file(path: String, content: String) -> Result<(), String> {
  let env = Env::current()?;
  blocking("write_instruction_file", move || {
    instruction_service::write_instruction_file(&env, &path, &content)
  })
  .await
}

#[tauri::command]
pub async fn create_instruction(
  name: String,
  content: Option<String>,
) -> Result<InstructionImportOutcome, String> {
  let env = Env::current()?;
  blocking("create_instruction", move || {
    instruction_service::create_instruction(&env, &name, content.as_deref().unwrap_or(""))
  })
  .await
}

#[tauri::command]
pub async fn install_instruction(
  request: InstallRequest,
) -> Result<InstructionActionResult, String> {
  let env = Env::current()?;
  let mut request = request;
  if request.mode.is_none() {
    let config = load_config()?;
    request.mode = Some(InstallMode::from_setting(&config.sync_mode));
  }
  blocking("install_instruction", move || {
    instruction_service::install_instruction(&env, request)
  })
  .await
}

#[tauri::command]
pub async fn uninstall_instruction(
  request: UninstallRequest,
) -> Result<InstructionActionResult, String> {
  let env = Env::current()?;
  blocking("uninstall_instruction", move || {
    instruction_service::uninstall_instruction(&env, request)
  })
  .await
}

#[tauri::command]
pub async fn remove_instruction(id: String, remove_installs: bool) -> Result<(), String> {
  let env = Env::current()?;
  blocking("remove_instruction", move || {
    instruction_service::remove_instruction(&env, &id, remove_installs)
  })
  .await
}

#[tauri::command]
pub async fn sync_instruction(
  id: String,
  action: SyncAction,
) -> Result<InstructionActionResult, String> {
  let env = Env::current()?;
  blocking("sync_instruction", move || {
    instruction_service::sync_instruction(&env, &id, action)
  })
  .await
}

#[tauri::command]
pub async fn diff_instruction(id: String, path: String) -> Result<SkillDiff, String> {
  let env = Env::current()?;
  blocking("diff_instruction", move || {
    instruction_service::diff_instruction(&env, &id, &path)
  })
  .await
}

#[tauri::command]
pub async fn write_instruction(
  id: String,
  content: String,
) -> Result<InstructionActionResult, String> {
  let env = Env::current()?;
  blocking("write_instruction", move || {
    instruction_service::write_instruction(&env, &id, &content)
  })
  .await
}

#[tauri::command]
pub async fn detect_instruction_files(
  paths: Vec<String>,
) -> Result<Vec<DetectedInstruction>, String> {
  let env = Env::current()?;
  blocking("detect_instruction_files", move || {
    instruction_service::detect_instruction_files(&env, &paths)
  })
  .await
}

#[tauri::command]
pub async fn detect_instruction_github(
  github_path: String,
) -> Result<Vec<DetectedInstruction>, String> {
  let env = Env::current()?;
  instruction_service::detect_instruction_github(&env, &github_path).await
}
