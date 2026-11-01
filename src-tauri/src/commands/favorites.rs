use crate::models::{FavoriteSnapshot, LocalFavorite};
use crate::services::favorites_service;

#[tauri::command]
pub fn list_favorites() -> Result<Vec<LocalFavorite>, String> {
  favorites_service::list_favorites()
}

/// Adds or removes a favorite and returns the updated list. An add needs the snapshot.
#[tauri::command]
pub fn set_favorite(
  skill_id: String,
  favorited: bool,
  snapshot: Option<FavoriteSnapshot>,
) -> Result<Vec<LocalFavorite>, String> {
  favorites_service::set_favorite(&skill_id, favorited, snapshot)
}
