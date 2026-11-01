use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AgentApp {
  pub id: String,
  pub display_name: String,
  pub project_path: Option<String>,
  pub global_path: Option<String>,
  /// Directory whose presence means the app is installed (e.g. `~/.cursor`). Needed when
  /// `global_path` is the shared `~/.agents/skills`, which says nothing about the app.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detect_path: Option<String>,
  /// Instruction file the agent reads inside a project (`AGENTS.md`, `CLAUDE.md`). Its
  /// presence marks a folder as a project.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub profile_path: Option<String>,
  /// User-level instruction file (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`).
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub global_profile_path: Option<String>,
  #[serde(default)]
  pub is_user_custom: bool,
}

/// An agent instruction file ("memory") of one install scope. Agents that read the same
/// file share one entry.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFile {
  /// Absolute path of the file.
  pub path: String,
  /// File name (`AGENTS.md`, `CLAUDE.md`).
  pub name: String,
  pub agent_ids: Vec<String>,
  pub exists: bool,
  /// Where the file links to when it is a symlink, `None` for a regular file.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub link_target: Option<String>,
  /// Files a pure reference file imports (`@AGENTS.md`); empty for any other file.
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub references: Vec<String>,
}
