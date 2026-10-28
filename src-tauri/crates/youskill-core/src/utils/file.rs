use serde_yaml::Value;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct SkillFrontmatter {
  pub name: Option<String>,
  pub description: Option<String>,
}

pub struct FileHelper;

impl FileHelper {
  pub fn read_to_string(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| e.to_string())
  }

  pub fn read_skill_frontmatter(skill_md_path: &Path) -> Result<SkillFrontmatter, String> {
    let content = Self::read_to_string(skill_md_path)?;
    if content.lines().next().map(|s| s.trim()) != Some("---") {
      return Err("SKILL.md frontmatter not found".to_string());
    }
    let (frontmatter, _) =
      split_frontmatter(&content).ok_or_else(|| "SKILL.md frontmatter not closed".to_string())?;
    parse_skill_frontmatter(frontmatter)
  }
}

/// Split a Markdown file into its `---` frontmatter and the body after it. `None` when the
/// file does not start with a closed frontmatter block.
pub fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
  let mut lines = content.split_inclusive('\n');
  let first = lines.next()?;
  if first.trim() != "---" {
    return None;
  }
  let start = first.len();
  let mut offset = start;
  for line in lines {
    if line.trim() == "---" {
      return Some((&content[start..offset], &content[offset + line.len()..]));
    }
    offset += line.len();
  }
  None
}

pub fn parse_skill_frontmatter(frontmatter: &str) -> Result<SkillFrontmatter, String> {
  let yaml: Value = serde_yaml::from_str(frontmatter)
    .map_err(|e| format!("Failed to parse SKILL.md frontmatter: {}", e))?;
  let read_field = |key: &str| {
    yaml
      .get(key)
      .and_then(|v| v.as_str())
      .map(|s| s.trim().to_string())
      .filter(|s| !s.is_empty())
  };
  Ok(SkillFrontmatter {
    name: read_field("name"),
    description: read_field("description"),
  })
}

pub fn open_in_file_manager(file_path: String) -> Result<(), String> {
  let path = Path::new(&file_path);
  if !path.exists() {
    return Err(format!("Path does not exist: {}", file_path));
  }

  #[cfg(target_os = "macos")]
  {
    Command::new("open")
      .arg("-R")
      .arg(&file_path)
      .spawn()
      .map_err(|e| format!("Failed to open file manager: {}", e))?;
    Ok(())
  }

  #[cfg(target_os = "linux")]
  {
    let managers = ["nautilus", "dolphin", "thunar", "pcmanfm"];
    let mut opened = false;
    for manager in managers {
      if Command::new(manager)
        .arg("--select")
        .arg(&file_path)
        .spawn()
        .is_ok()
      {
        opened = true;
        break;
      }
    }
    if !opened {
      Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn()
        .map_err(|e| format!("Failed to open file manager: {}", e))?;
    }
    Ok(())
  }

  #[cfg(target_os = "windows")]
  {
    Command::new("explorer")
      .arg("/select,")
      .arg(&file_path)
      .spawn()
      .map_err(|e| format!("Failed to open file manager: {}", e))?;
    Ok(())
  }
}

pub async fn read_skill_file(skill_path: String) -> Result<String, String> {
  let path = Path::new(&skill_path);
  if !path.exists() {
    return Err(format!("Skill directory does not exist: {}", skill_path));
  }

  let skill_md = path.join("SKILL.md");
  if skill_md.exists() {
    return tokio::fs::read_to_string(&skill_md)
      .await
      .map_err(|e| format!("Failed to read SKILL.md: {}", e));
  }

  Err(format!("SKILL.md not found in: {}", skill_path))
}
