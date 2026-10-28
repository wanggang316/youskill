use crate::models::{RemoteSkill, RemoteSkillsResponse};
use crate::services::remote_service;

#[tauri::command]
pub async fn fetch_remote_skills(
  skip: Option<u32>,
  limit: Option<u32>,
  search: Option<String>,
  sort_by: Option<String>,
  sort_order: Option<String>,
  tag_id: Option<u64>,
  source: Option<String>,
) -> Result<RemoteSkillsResponse, String> {
  remote_service::fetch_remote_skills(skip, limit, search, sort_by, sort_order, tag_id, source)
    .await
}

#[tauri::command]
pub async fn fetch_skills_by_names(names: Vec<String>) -> Result<Vec<RemoteSkill>, String> {
  remote_service::fetch_skills_by_names(names).await
}

#[tauri::command]
pub async fn record_skill_install(skill_id: String) -> Result<(), String> {
  remote_service::record_skill_install(skill_id).await
}

#[tauri::command]
pub async fn fetch_market_categories() -> Result<Vec<remote_service::MarketCategory>, String> {
  remote_service::fetch_market_categories().await
}

#[tauri::command]
pub async fn fetch_hot_repositories() -> Result<Vec<remote_service::MarketRepository>, String> {
  remote_service::fetch_hot_repositories().await
}

#[tauri::command]
pub async fn fetch_remote_skill(skill_id: String) -> Result<RemoteSkill, String> {
  remote_service::fetch_remote_skill(skill_id).await
}
