use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const TRANSLATE_DISPLAY_MODES: [&str; 2] = ["bilingual", "translation"];
pub const DEFAULT_TRANSLATE_DISPLAY_MODE: &str = TRANSLATE_DISPLAY_MODES[0];
pub const TRANSLATE_TEXT_STYLES: [&str; 2] = ["none", "dashed_underline"];
pub const DEFAULT_TRANSLATE_TEXT_STYLE: &str = TRANSLATE_TEXT_STYLES[0];
/// Actions of the top bar that accept a keyboard shortcut.
pub const SHORTCUT_ACTIONS: [&str; 7] = [
  "library",
  "instructions",
  "projects",
  "market",
  "import",
  "search",
  "settings",
];

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct AppConfig {
  pub language: String,
  pub theme: String,
  /// Default install mode: "copy" or "symlink".
  pub sync_mode: String,
  pub backup_folder: Option<String>,
  pub last_backup_time: Option<String>,
  pub openrouter_api_key: Option<String>,
  pub translate_target_language: String,
  pub translate_model: String,
  /// Translated markdown display mode: "bilingual" or "translation".
  pub translate_display_mode: String,
  /// Translated text style: "none" or "dashed_underline".
  pub translate_text_style: String,
  /// GitHub CLI account used for GitHub requests. `None` follows the active `gh` account.
  pub github_account: Option<String>,
  /// Shortcuts the user changed, by action. Actions not listed use their default key.
  pub shortcuts: BTreeMap<String, String>,
}

impl Default for AppConfig {
  fn default() -> Self {
    Self {
      language: "en".to_string(),
      theme: "system".to_string(),
      sync_mode: "copy".to_string(),
      backup_folder: None,
      last_backup_time: None,
      openrouter_api_key: None,
      translate_target_language: String::new(),
      translate_model: String::new(),
      translate_display_mode: DEFAULT_TRANSLATE_DISPLAY_MODE.to_string(),
      translate_text_style: DEFAULT_TRANSLATE_TEXT_STYLE.to_string(),
      github_account: None,
      shortcuts: BTreeMap::new(),
    }
  }
}

pub fn config_path() -> Result<PathBuf, String> {
  let config_dir = dirs_next::config_dir().ok_or("无法获取配置目录")?;
  Ok(config_dir.join("youskill").join("youskill.config"))
}

pub fn load_config() -> Result<AppConfig, String> {
  let path = config_path()?;
  if !path.exists() {
    return Ok(AppConfig::default());
  }
  let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
  serde_json::from_str(&content).map_err(|e| e.to_string())
}

pub fn save_config(config: &AppConfig) -> Result<(), String> {
  let path = config_path()?;
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  let content = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
  fs::write(&path, content).map_err(|e| e.to_string())
}
