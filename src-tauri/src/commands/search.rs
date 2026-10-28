use crate::services::env::Env;
use crate::services::search_service::{self, SearchGroup};

#[tauri::command]
pub async fn global_search(query: String, user_name: String) -> Result<Vec<SearchGroup>, String> {
  let env = Env::current()?;
  tauri::async_runtime::spawn_blocking(move || search_service::search(&env, &query, &user_name))
    .await
    .map_err(|e| format!("global_search join error: {}", e))?
}
