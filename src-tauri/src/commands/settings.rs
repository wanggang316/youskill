use crate::config::{
  load_config, save_config, DEFAULT_TRANSLATE_DISPLAY_MODE, DEFAULT_TRANSLATE_TEXT_STYLE,
  SHORTCUT_ACTIONS, TRANSLATE_DISPLAY_MODES, TRANSLATE_TEXT_STYLES,
};
use crate::services::ai_service::{self, OpenRouterModelOption};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Upper bound for one stored key combination, e.g. "Mod+Alt+Shift+ArrowDown".
const MAX_SHORTCUT_LEN: usize = 64;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettingsPayload {
  pub language: String,
  pub theme: String,
  pub sync_mode: String,
  #[serde(default)]
  pub openrouter_api_key: Option<String>,
  #[serde(default = "default_translate_target_language")]
  pub translate_target_language: String,
  #[serde(default = "default_translate_model")]
  pub translate_model: String,
  #[serde(default = "default_translate_display_mode")]
  pub translate_display_mode: String,
  #[serde(default = "default_translate_text_style")]
  pub translate_text_style: String,
  #[serde(default)]
  pub shortcuts: BTreeMap<String, String>,
}

fn default_translate_target_language() -> String {
  String::new()
}

fn default_translate_model() -> String {
  String::new()
}

fn default_translate_display_mode() -> String {
  DEFAULT_TRANSLATE_DISPLAY_MODE.to_string()
}

fn default_translate_text_style() -> String {
  DEFAULT_TRANSLATE_TEXT_STYLE.to_string()
}

/// Returns `value` when it is one of `allowed`, otherwise `fallback`.
fn normalize_choice(value: &str, allowed: &[&str], fallback: &str) -> String {
  let value = value.trim();
  if allowed.contains(&value) {
    value.to_string()
  } else {
    fallback.to_string()
  }
}

/// Keeps the overrides of known actions that hold a usable key combination.
fn normalize_shortcuts(shortcuts: &BTreeMap<String, String>) -> BTreeMap<String, String> {
  shortcuts
    .iter()
    .filter(|(action, _)| SHORTCUT_ACTIONS.contains(&action.as_str()))
    .map(|(action, binding)| (action.clone(), binding.trim().to_string()))
    .filter(|(_, binding)| !binding.is_empty() && binding.len() <= MAX_SHORTCUT_LEN)
    .collect()
}

// `async` runs it off the main thread; the window requests it during startup.
#[tauri::command(async)]
pub fn get_settings() -> Result<SettingsPayload, String> {
  let config = load_config()?;
  Ok(SettingsPayload {
    language: config.language,
    theme: config.theme,
    sync_mode: config.sync_mode,
    openrouter_api_key: config.openrouter_api_key,
    translate_target_language: config.translate_target_language,
    translate_model: config.translate_model,
    translate_display_mode: normalize_choice(
      &config.translate_display_mode,
      &TRANSLATE_DISPLAY_MODES,
      DEFAULT_TRANSLATE_DISPLAY_MODE,
    ),
    translate_text_style: normalize_choice(
      &config.translate_text_style,
      &TRANSLATE_TEXT_STYLES,
      DEFAULT_TRANSLATE_TEXT_STYLE,
    ),
    shortcuts: normalize_shortcuts(&config.shortcuts),
  })
}

#[tauri::command]
pub fn update_settings(settings: SettingsPayload) -> Result<SettingsPayload, String> {
  let mut config = load_config()?;
  config.language = settings.language.clone();
  config.theme = settings.theme.clone();
  config.sync_mode = settings.sync_mode.clone();
  config.openrouter_api_key = settings
    .openrouter_api_key
    .as_ref()
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
  config.translate_target_language = if settings.translate_target_language.trim().is_empty() {
    default_translate_target_language()
  } else {
    settings.translate_target_language.trim().to_string()
  };
  config.translate_model = if settings.translate_model.trim().is_empty() {
    default_translate_model()
  } else {
    settings.translate_model.trim().to_string()
  };
  config.translate_display_mode = normalize_choice(
    &settings.translate_display_mode,
    &TRANSLATE_DISPLAY_MODES,
    DEFAULT_TRANSLATE_DISPLAY_MODE,
  );
  config.translate_text_style = normalize_choice(
    &settings.translate_text_style,
    &TRANSLATE_TEXT_STYLES,
    DEFAULT_TRANSLATE_TEXT_STYLE,
  );
  config.shortcuts = normalize_shortcuts(&settings.shortcuts);
  save_config(&config)?;
  Ok(SettingsPayload {
    language: config.language,
    theme: config.theme,
    sync_mode: config.sync_mode,
    openrouter_api_key: config.openrouter_api_key,
    translate_target_language: config.translate_target_language,
    translate_model: config.translate_model,
    translate_display_mode: normalize_choice(
      &config.translate_display_mode,
      &TRANSLATE_DISPLAY_MODES,
      DEFAULT_TRANSLATE_DISPLAY_MODE,
    ),
    translate_text_style: normalize_choice(
      &config.translate_text_style,
      &TRANSLATE_TEXT_STYLES,
      DEFAULT_TRANSLATE_TEXT_STYLE,
    ),
    shortcuts: normalize_shortcuts(&config.shortcuts),
  })
}

#[tauri::command]
pub async fn list_openrouter_models(
  search: Option<String>,
) -> Result<Vec<OpenRouterModelOption>, String> {
  let config = load_config()?;
  ai_service::list_openrouter_models(config.openrouter_api_key.as_deref(), search).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn normalize_choice_keeps_allowed_values() {
    assert_eq!(
      normalize_choice(" translation ", &TRANSLATE_DISPLAY_MODES, "bilingual"),
      "translation"
    );
    assert_eq!(
      normalize_choice("dashed_underline", &TRANSLATE_TEXT_STYLES, "none"),
      "dashed_underline"
    );
  }

  #[test]
  fn normalize_choice_falls_back_for_unknown_values() {
    assert_eq!(
      normalize_choice("", &TRANSLATE_DISPLAY_MODES, DEFAULT_TRANSLATE_DISPLAY_MODE),
      "bilingual"
    );
    assert_eq!(
      normalize_choice("wavy", &TRANSLATE_TEXT_STYLES, DEFAULT_TRANSLATE_TEXT_STYLE),
      "none"
    );
  }

  #[test]
  fn normalize_shortcuts_drops_unknown_actions_and_blank_keys() {
    let shortcuts = BTreeMap::from([
      ("library".to_string(), " Mod+l ".to_string()),
      ("market".to_string(), "  ".to_string()),
      ("unknown".to_string(), "x".to_string()),
      ("import".to_string(), "k".repeat(MAX_SHORTCUT_LEN + 1)),
    ]);
    assert_eq!(
      normalize_shortcuts(&shortcuts),
      BTreeMap::from([("library".to_string(), "Mod+l".to_string())])
    );
  }

  #[test]
  fn settings_payload_defaults_translate_view_options() {
    let payload: SettingsPayload =
      serde_json::from_str(r#"{"language":"en","theme":"system","sync_mode":"copy"}"#).unwrap();
    assert_eq!(payload.translate_display_mode, "bilingual");
    assert_eq!(payload.translate_text_style, "none");
    assert!(payload.shortcuts.is_empty());
  }

  #[test]
  fn app_config_defaults_translate_view_options_for_old_files() {
    let config: crate::config::AppConfig = serde_json::from_str(r#"{"language":"zh"}"#).unwrap();
    assert_eq!(config.translate_display_mode, "bilingual");
    assert_eq!(config.translate_text_style, "none");
    assert!(config.shortcuts.is_empty());
  }
}
