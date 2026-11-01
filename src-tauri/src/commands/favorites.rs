use crate::favorite_sync;
use crate::models::{FavoriteSnapshot, LocalFavorite};
use crate::services::favorites_service;
use tauri::AppHandle;

#[tauri::command]
pub fn list_favorites() -> Result<Vec<LocalFavorite>, String> {
  favorites_service::list_favorites()
}

/// Adds or removes a favorite and returns the updated list. An add needs the snapshot. The
/// change syncs with the account shortly after.
#[tauri::command]
pub fn set_favorite(
  app: AppHandle,
  skill_id: String,
  favorited: bool,
  snapshot: Option<FavoriteSnapshot>,
) -> Result<Vec<LocalFavorite>, String> {
  let list = favorites_service::set_favorite(&skill_id, favorited, snapshot)?;
  favorite_sync::request(&app, favorite_sync::AFTER_CHANGE);
  Ok(list)
}
