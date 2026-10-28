//! Global Search: one query over hub skills, instruction templates and install scopes.
//! Names match as a fuzzy subsequence; descriptions, paths and bodies as a substring. All
//! matching ignores case. Bodies are read from disk and cached by modification time.

use crate::services::env::Env;
use crate::services::instruction_service::read_lock;
use crate::services::lock_service::store;
use crate::utils::file::{parse_skill_frontmatter, split_frontmatter};
use crate::utils::folder::SKILL_MD;
use crate::utils::path::path_to_string;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

pub const MAX_HITS_PER_GROUP: usize = 8;
/// Length of a snippet in characters, without the ellipses.
const SNIPPET_CHARS: usize = 80;
/// The user scope also matches this name in every app language.
const USER_SCOPE_NAME: &str = "User";

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchGroupKind {
  Skills,
  Instructions,
  Projects,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchItemKind {
  Skill,
  Instruction,
  Project,
  /// The user scope; it has no id.
  User,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchField {
  Name,
  Description,
  Path,
  Body,
}

/// A part of a text, as `[start, end)` in characters (Unicode scalar values).
pub type CharRange = [usize; 2];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
  pub text: String,
  pub range: CharRange,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
  pub kind: SearchItemKind,
  /// What opens the item: skill id, template id or project path. Empty for the user scope.
  pub id: String,
  pub name: String,
  pub field: MatchField,
  /// Matched characters of `name`.
  pub name_ranges: Vec<CharRange>,
  /// Second line: skill description, or scope path.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  /// Text around a description, path or body match.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snippet: Option<Snippet>,
  /// The project folder no longer exists.
  pub missing: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchGroup {
  pub kind: SearchGroupKind,
  pub hits: Vec<SearchHit>,
}

/// Search every kind of item for `query`. `user_name` is the label of the user scope in the
/// app language; the user scope also matches "User". Groups come in the order skills,
/// instructions, projects, hold at most [`MAX_HITS_PER_GROUP`] hits and are left out when
/// empty. A blank query returns no groups.
pub fn search(env: &Env, query: &str, user_name: &str) -> Result<Vec<SearchGroup>, String> {
  let query = fold(query.trim());
  if query.is_empty() {
    return Ok(Vec::new());
  }
  let groups = [
    (SearchGroupKind::Skills, skill_hits(env, &query)?),
    (
      SearchGroupKind::Instructions,
      instruction_hits(env, &query)?,
    ),
    (
      SearchGroupKind::Projects,
      scope_hits(env, &query, user_name),
    ),
  ];
  Ok(
    groups
      .into_iter()
      .filter(|(_, hits)| !hits.is_empty())
      .map(|(kind, hits)| SearchGroup { kind, hits })
      .collect(),
  )
}

fn skill_hits(env: &Env, query: &[char]) -> Result<Vec<SearchHit>, String> {
  let lock = store(env).read()?;
  let mut records: Vec<(&String, &str)> = lock
    .skills
    .iter()
    .filter(|(_, record)| !record.name.is_empty())
    .map(|(id, record)| (id, record.name.as_str()))
    .collect();
  records.sort_by_cached_key(|(id, name)| (name.to_lowercase(), id.to_string()));

  let mut ranked = Vec::new();
  for (id, name) in records {
    let doc = read_doc(&env.hub_dir(id, name).join(SKILL_MD));
    let description = doc.as_ref().and_then(|doc| doc.description.as_deref());
    let candidate = Candidate {
      kind: SearchItemKind::Skill,
      id,
      names: &[name],
      detail: description,
      fields: &[(MatchField::Description, description)],
      body: doc.as_ref().map(|doc| doc.body.as_str()),
      missing: false,
    };
    ranked.extend(candidate.rank(query));
  }
  Ok(top(ranked))
}

fn instruction_hits(env: &Env, query: &[char]) -> Result<Vec<SearchHit>, String> {
  let lock = read_lock(env)?;
  let mut records: Vec<(&String, &str)> = lock
    .instructions
    .iter()
    .map(|(id, record)| (id, record.name.as_str()))
    .collect();
  records.sort_by_cached_key(|(id, name)| (name.to_lowercase(), id.to_string()));

  let mut ranked = Vec::new();
  for (id, name) in records {
    let doc = read_doc(&env.instruction_file(id));
    let candidate = Candidate {
      kind: SearchItemKind::Instruction,
      id,
      names: &[name],
      detail: None,
      fields: &[],
      body: doc.as_ref().map(|doc| doc.body.as_str()),
      missing: false,
    };
    ranked.extend(candidate.rank(query));
  }
  Ok(top(ranked))
}

/// The user scope, then every registered project in list order.
fn scope_hits(env: &Env, query: &[char], user_name: &str) -> Vec<SearchHit> {
  let home = path_to_string(&env.home);
  let user_name = if user_name.trim().is_empty() {
    USER_SCOPE_NAME
  } else {
    user_name
  };
  let user = Candidate {
    kind: SearchItemKind::User,
    id: "",
    names: &[user_name, USER_SCOPE_NAME],
    detail: Some(&home),
    fields: &[],
    body: None,
    missing: false,
  };
  let mut ranked: Vec<_> = user.rank(query).into_iter().collect();
  for project in &env.projects {
    let path = Some(project.path.as_str());
    let candidate = Candidate {
      kind: SearchItemKind::Project,
      id: &project.path,
      names: &[project.name.as_str()],
      detail: path,
      fields: &[(MatchField::Path, path)],
      body: None,
      missing: !Path::new(&project.path).is_dir(),
    };
    ranked.extend(candidate.rank(query));
  }
  top(ranked)
}

/// Best hits first; equal ranks keep their order.
fn top(mut ranked: Vec<(Rank, SearchHit)>) -> Vec<SearchHit> {
  ranked.sort_by_key(|(rank, _)| *rank);
  ranked
    .into_iter()
    .take(MAX_HITS_PER_GROUP)
    .map(|(_, hit)| hit)
    .collect()
}

/// Lower is better: the match tier, then how tight a fuzzy name match is.
type Rank = (u8, usize);

const TIER_EXACT: u8 = 0;
const TIER_PREFIX: u8 = 1;
const TIER_FUZZY: u8 = 2;
const TIER_DESCRIPTION: u8 = 3;
const TIER_PATH: u8 = 4;
const TIER_BODY: u8 = 5;

struct Candidate<'a> {
  kind: SearchItemKind,
  id: &'a str,
  /// The first name is shown; the others only match.
  names: &'a [&'a str],
  detail: Option<&'a str>,
  /// Text fields that match as a substring, before the body.
  fields: &'a [(MatchField, Option<&'a str>)],
  /// Body with whitespace collapsed, see [`Doc`].
  body: Option<&'a str>,
  missing: bool,
}

impl Candidate<'_> {
  fn rank(&self, query: &[char]) -> Option<(Rank, SearchHit)> {
    let hit = |field, name_ranges, snippet| SearchHit {
      kind: self.kind,
      id: self.id.to_string(),
      name: self.names[0].to_string(),
      field,
      name_ranges,
      detail: self.detail.map(str::to_string),
      snippet,
      missing: self.missing,
    };

    let shown = match_name(self.names[0], query);
    let best = self.names[1..]
      .iter()
      .filter_map(|name| match_name(name, query).map(|(rank, _)| rank))
      .chain(shown.as_ref().map(|(rank, _)| *rank))
      .min();
    if let Some(rank) = best {
      // Mark the shown name only; ranges of another name do not apply to it.
      let ranges = shown.map(|(_, ranges)| ranges).unwrap_or_default();
      return Some((rank, hit(MatchField::Name, ranges, None)));
    }

    for (field, text) in self.fields {
      let Some(text) = text else { continue };
      let tier = match field {
        MatchField::Path => TIER_PATH,
        _ => TIER_DESCRIPTION,
      };
      let text = collapse_whitespace(text);
      if let Some(snippet) = find_snippet(&text, query) {
        return Some(((tier, 0), hit(*field, Vec::new(), Some(snippet))));
      }
    }

    let snippet = find_snippet(self.body?, query)?;
    Some((
      (TIER_BODY, 0),
      hit(MatchField::Body, Vec::new(), Some(snippet)),
    ))
  }
}

/// Exact, prefix or fuzzy subsequence match of a name, with the matched ranges.
fn match_name(name: &str, query: &[char]) -> Option<(Rank, Vec<CharRange>)> {
  let name = fold(name);
  let len = query.len();
  if name == query {
    return Some(((TIER_EXACT, 0), vec![[0, len]]));
  }
  if name.starts_with(query) {
    return Some(((TIER_PREFIX, 0), vec![[0, len]]));
  }
  if let Some(start) = find(&name, query) {
    return Some(((TIER_FUZZY, 0), vec![[start, start + len]]));
  }

  let mut positions = Vec::with_capacity(len);
  let mut next = 0;
  for wanted in query {
    let offset = name[next..].iter().position(|c| c == wanted)?;
    positions.push(next + offset);
    next += offset + 1;
  }
  let span = positions[len - 1] - positions[0] + 1;
  let mut ranges: Vec<CharRange> = Vec::new();
  for position in positions {
    match ranges.last_mut() {
      Some(last) if last[1] == position => last[1] += 1,
      _ => ranges.push([position, position + 1]),
    }
  }
  Some(((TIER_FUZZY, span), ranges))
}

/// About [`SNIPPET_CHARS`] characters around the first match in `text`, with an ellipsis
/// where text was cut off.
fn find_snippet(text: &str, query: &[char]) -> Option<Snippet> {
  let chars: Vec<char> = text.chars().collect();
  let folded: Vec<char> = chars.iter().map(|c| fold_char(*c)).collect();
  let start = find(&folded, query)?;
  let end = start + query.len();

  let room = SNIPPET_CHARS.saturating_sub(query.len());
  let mut from = start.saturating_sub(room / 2);
  let to = (from + SNIPPET_CHARS).max(end).min(chars.len());
  from = from.min(to.saturating_sub(SNIPPET_CHARS));

  let mut snippet = String::new();
  let mut offset = 0;
  if from > 0 {
    snippet.push('…');
    offset = 1;
  }
  snippet.extend(&chars[from..to]);
  if to < chars.len() {
    snippet.push('…');
  }
  Some(Snippet {
    text: snippet,
    range: [start - from + offset, end - from + offset],
  })
}

fn find(haystack: &[char], needle: &[char]) -> Option<usize> {
  if needle.len() > haystack.len() {
    return None;
  }
  haystack
    .windows(needle.len())
    .position(|window| window == needle)
}

/// Lower case per character, so that positions in the folded text are positions in the
/// original text.
fn fold_char(c: char) -> char {
  c.to_lowercase().next().unwrap_or(c)
}

fn fold(text: &str) -> Vec<char> {
  text.chars().map(fold_char).collect()
}

/// One line: every run of whitespace becomes one space.
fn collapse_whitespace(text: &str) -> String {
  text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

/// What search reads from a Markdown file.
struct Doc {
  description: Option<String>,
  /// Text after the frontmatter, on one line.
  body: String,
}

/// File modification time and length; a change to either reads the file again.
type FileStamp = (SystemTime, u64);

type DocCache = HashMap<PathBuf, (FileStamp, Arc<Doc>)>;

static DOC_CACHE: RwLock<Option<DocCache>> = RwLock::new(None);

fn read_doc(path: &Path) -> Option<Arc<Doc>> {
  let metadata = fs::metadata(path).ok()?;
  let stamp = (metadata.modified().ok()?, metadata.len());
  if let Ok(cache) = DOC_CACHE.read() {
    if let Some((cached, doc)) = cache.as_ref().and_then(|map| map.get(path)) {
      if *cached == stamp {
        return Some(doc.clone());
      }
    }
  }

  let content = fs::read_to_string(path).ok()?;
  let (description, body) = match split_frontmatter(&content) {
    Some((frontmatter, body)) => (
      parse_skill_frontmatter(frontmatter)
        .ok()
        .and_then(|frontmatter| frontmatter.description),
      body,
    ),
    None => (None, content.as_str()),
  };
  let doc = Arc::new(Doc {
    description,
    body: collapse_whitespace(body),
  });
  if let Ok(mut cache) = DOC_CACHE.write() {
    cache
      .get_or_insert_with(HashMap::new)
      .insert(path.to_path_buf(), (stamp, doc.clone()));
  }
  Some(doc)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::models::{ImportItem, InstructionImportItem, SkillSource, UserProject};
  use crate::services::hub_service::import_skills;
  use crate::services::instruction_service::import_instructions;
  use std::fs;
  use std::path::Path;

  fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
  }

  fn project(path: &Path) -> UserProject {
    UserProject {
      name: path.file_name().unwrap().to_string_lossy().to_string(),
      path: path.to_string_lossy().to_string(),
      workspace_path: None,
    }
  }

  fn env(tmp: &Path, projects: Vec<UserProject>) -> Env {
    Env::for_test(tmp, vec![], projects)
  }

  /// Import a skill with the given description and body; returns its id.
  fn skill(env: &Env, tmp: &Path, name: &str, description: &str, body: &str) -> String {
    let dir = tmp.join("staged").join(new_dir_name());
    write(
      &dir.join("SKILL.md"),
      &format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
    );
    let item = ImportItem {
      name: name.to_string(),
      tmp_path: dir.to_string_lossy().to_string(),
      source: SkillSource::Folder {
        path: dir.to_string_lossy().to_string(),
      },
      replace_id: None,
    };
    import_skills(env, vec![item], false).unwrap().remove(0).id
  }

  /// Import an instruction template; returns its id.
  fn template(env: &Env, tmp: &Path, name: &str, body: &str) -> String {
    let src = tmp.join("src").join(format!("{}.md", new_dir_name()));
    write(&src, body);
    import_instructions(
      env,
      vec![InstructionImportItem {
        name: name.to_string(),
        path: src.to_string_lossy().to_string(),
        source: None,
      }],
    )
    .unwrap()
    .remove(0)
    .id
  }

  fn new_dir_name() -> String {
    crate::models::new_skill_id()
  }

  fn run(env: &Env, query: &str) -> Vec<SearchGroup> {
    search(env, query, "User").unwrap()
  }

  fn group(groups: &[SearchGroup], kind: SearchGroupKind) -> Vec<SearchHit> {
    groups
      .iter()
      .find(|group| group.kind == kind)
      .map(|group| group.hits.clone())
      .unwrap_or_default()
  }

  fn skills(env: &Env, query: &str) -> Vec<SearchHit> {
    group(&run(env, query), SearchGroupKind::Skills)
  }

  fn names(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter().map(|hit| hit.name.as_str()).collect()
  }

  fn highlighted(text: &str, range: CharRange) -> String {
    text
      .chars()
      .skip(range[0])
      .take(range[1] - range[0])
      .collect()
  }

  #[test]
  fn blank_query_returns_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    skill(&env, tmp.path(), "git-skill", "Git helper", "body");
    assert!(run(&env, "").is_empty());
    assert!(run(&env, "   ").is_empty());
  }

  #[test]
  fn skill_found_by_name_description_and_body() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    let id = skill(
      &env,
      tmp.path(),
      "git-skill",
      "Helps with rebases",
      "Use cherry-pick.",
    );

    let hits = skills(&env, "git");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchItemKind::Skill);
    assert_eq!(hits[0].id, id);
    assert_eq!(hits[0].field, MatchField::Name);
    assert_eq!(hits[0].name_ranges, vec![[0, 3]]);
    assert_eq!(hits[0].detail.as_deref(), Some("Helps with rebases"));

    let hits = skills(&env, "rebases");
    assert_eq!(hits[0].field, MatchField::Description);
    let snippet = hits[0].snippet.as_ref().unwrap();
    assert_eq!(highlighted(&snippet.text, snippet.range), "rebases");

    let hits = skills(&env, "cherry");
    assert_eq!(hits[0].field, MatchField::Body);
    let snippet = hits[0].snippet.as_ref().unwrap();
    assert_eq!(highlighted(&snippet.text, snippet.range), "cherry");
  }

  #[test]
  fn name_matching_is_fuzzy_and_ignores_case() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    skill(&env, tmp.path(), "git-skill", "d", "b");

    let hits = skills(&env, "GSK");
    assert_eq!(names(&hits), vec!["git-skill"]);
    assert_eq!(hits[0].field, MatchField::Name);
    assert_eq!(hits[0].name_ranges, vec![[0, 1], [4, 6]]);
    assert!(skills(&env, "gxk").is_empty());
  }

  #[test]
  fn frontmatter_is_not_a_body_match() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    skill(&env, tmp.path(), "alpha", "first", "plain text");
    assert!(skills(&env, "name:").is_empty());
    assert!(skills(&env, "description").is_empty());
  }

  #[test]
  fn ranking_puts_names_before_content() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    skill(&env, tmp.path(), "body-hit", "d", "mentions deploy here");
    skill(&env, tmp.path(), "desc-hit", "deploy things", "b");
    skill(&env, tmp.path(), "do-every-practical-loop-yes", "d", "b");
    skill(&env, tmp.path(), "deploy-tools", "d", "b");
    skill(&env, tmp.path(), "deploy", "d", "b");

    let hits = skills(&env, "deploy");
    assert_eq!(
      names(&hits),
      vec![
        "deploy",
        "deploy-tools",
        "do-every-practical-loop-yes",
        "desc-hit",
        "body-hit"
      ]
    );
    let fields: Vec<MatchField> = hits.iter().map(|hit| hit.field).collect();
    assert_eq!(
      fields,
      vec![
        MatchField::Name,
        MatchField::Name,
        MatchField::Name,
        MatchField::Description,
        MatchField::Body
      ]
    );
  }

  #[test]
  fn same_name_skills_are_separate_hits() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    let first = skill(&env, tmp.path(), "tdd", "one", "b");
    let second = skill(&env, tmp.path(), "tdd", "two", "b");
    assert_ne!(first, second);

    let hits = skills(&env, "tdd");
    let mut ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
    ids.sort();
    let mut expected = vec![first.as_str(), second.as_str()];
    expected.sort();
    assert_eq!(ids, expected);
  }

  #[test]
  fn groups_are_capped_ordered_and_empty_ones_left_out() {
    let tmp = tempfile::tempdir().unwrap();
    let proj = tmp.path().join("work").join("notes-app");
    fs::create_dir_all(&proj).unwrap();
    let env = env(tmp.path(), vec![project(&proj)]);
    for index in 0..10 {
      skill(&env, tmp.path(), &format!("notes-{index}"), "d", "b");
    }
    template(&env, tmp.path(), "notes rules", "body");

    let groups = run(&env, "notes");
    let kinds: Vec<SearchGroupKind> = groups.iter().map(|group| group.kind).collect();
    assert_eq!(
      kinds,
      vec![
        SearchGroupKind::Skills,
        SearchGroupKind::Instructions,
        SearchGroupKind::Projects
      ]
    );
    assert_eq!(groups[0].hits.len(), MAX_HITS_PER_GROUP);

    let groups = run(&env, "rules");
    let kinds: Vec<SearchGroupKind> = groups.iter().map(|group| group.kind).collect();
    assert_eq!(kinds, vec![SearchGroupKind::Instructions]);
  }

  #[test]
  fn template_found_by_name_and_body() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    let id = template(
      &env,
      tmp.path(),
      "Team Rules",
      "# Rules\n\nAlways run the linter.\n",
    );

    let hits = group(&run(&env, "team"), SearchGroupKind::Instructions);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchItemKind::Instruction);
    assert_eq!(hits[0].id, id);
    assert_eq!(hits[0].field, MatchField::Name);

    let hits = group(&run(&env, "LINTER"), SearchGroupKind::Instructions);
    assert_eq!(hits[0].field, MatchField::Body);
    let snippet = hits[0].snippet.as_ref().unwrap();
    assert_eq!(highlighted(&snippet.text, snippet.range), "linter");
    assert!(!snippet.text.contains('\n'));
  }

  #[test]
  fn chinese_text_matches_as_substring() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    template(&env, tmp.path(), "团队规范", "提交前必须运行测试。");
    skill(&env, tmp.path(), "zh", "代码审查助手", "b");

    let hits = group(&run(&env, "规范"), SearchGroupKind::Instructions);
    assert_eq!(names(&hits), vec!["团队规范"]);
    assert_eq!(hits[0].name_ranges, vec![[2, 4]]);

    let hits = group(&run(&env, "运行测试"), SearchGroupKind::Instructions);
    assert_eq!(hits[0].field, MatchField::Body);
    let snippet = hits[0].snippet.as_ref().unwrap();
    assert_eq!(highlighted(&snippet.text, snippet.range), "运行测试");

    assert_eq!(skills(&env, "审查")[0].field, MatchField::Description);
  }

  #[test]
  fn snippet_is_short_and_cut_on_char_boundaries() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    let before = "前文".repeat(100);
    let after = "后文".repeat(100);
    template(
      &env,
      tmp.path(),
      "long",
      &format!("{before}\n\nneedle\n{after}"),
    );

    let hits = group(&run(&env, "needle"), SearchGroupKind::Instructions);
    let snippet = hits[0].snippet.as_ref().unwrap();
    let length = snippet.text.chars().count();
    assert!((70..=90).contains(&length), "snippet has {length} chars");
    assert_eq!(highlighted(&snippet.text, snippet.range), "needle");
    assert!(!snippet.text.contains('\n'));
  }

  #[test]
  fn file_changed_on_disk_is_found_by_next_search() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);
    let id = template(&env, tmp.path(), "rules", "old text");
    assert!(run(&env, "fresh").is_empty());

    // A different length makes the change visible even when the mtime does not move.
    fs::write(env.instruction_file(&id), "fresh text here").unwrap();
    let hits = group(&run(&env, "fresh"), SearchGroupKind::Instructions);
    assert_eq!(hits[0].id, id);

    let skill_id = skill(&env, tmp.path(), "alpha", "d", "old");
    let skill_md = env.hub_dir(&skill_id, "alpha").join("SKILL.md");
    fs::write(&skill_md, "---\nname: alpha\n---\nbrand new words").unwrap();
    assert_eq!(skills(&env, "brand new")[0].id, skill_id);
  }

  #[test]
  fn projects_found_by_name_and_path_and_flagged_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let present = tmp.path().join("clients").join("acme-web");
    fs::create_dir_all(&present).unwrap();
    let gone = tmp.path().join("old").join("legacy-api");
    let env = env(tmp.path(), vec![project(&present), project(&gone)]);

    let hits = group(&run(&env, "acme"), SearchGroupKind::Projects);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchItemKind::Project);
    assert_eq!(hits[0].id, present.to_string_lossy());
    assert_eq!(hits[0].field, MatchField::Name);
    assert!(!hits[0].missing);
    assert_eq!(hits[0].detail.as_deref(), Some(&*present.to_string_lossy()));

    let hits = group(&run(&env, "clients/"), SearchGroupKind::Projects);
    assert_eq!(names(&hits), vec!["acme-web"]);
    assert_eq!(hits[0].field, MatchField::Path);
    let snippet = hits[0].snippet.as_ref().unwrap();
    assert_eq!(highlighted(&snippet.text, snippet.range), "clients/");

    let hits = group(&run(&env, "legacy"), SearchGroupKind::Projects);
    assert!(hits[0].missing);
  }

  #[test]
  fn user_scope_found_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env(tmp.path(), vec![]);

    let hits = group(&run(&env, "user"), SearchGroupKind::Projects);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchItemKind::User);
    assert_eq!(hits[0].name, "User");
    assert_eq!(hits[0].name_ranges, vec![[0, 4]]);

    // The shown name marks its own match, also when the English name ranks higher.
    let groups = search(&env, "user", "User level").unwrap();
    let hits = group(&groups, SearchGroupKind::Projects);
    assert_eq!(hits[0].name, "User level");
    assert_eq!(hits[0].name_ranges, vec![[0, 4]]);

    let groups = search(&env, "用户", "用户").unwrap();
    let hits = group(&groups, SearchGroupKind::Projects);
    assert_eq!(hits[0].kind, SearchItemKind::User);
    assert_eq!(hits[0].name, "用户");
    // The English name keeps working in another app language.
    let groups = search(&env, "user", "用户").unwrap();
    assert_eq!(group(&groups, SearchGroupKind::Projects).len(), 1);
  }
}
