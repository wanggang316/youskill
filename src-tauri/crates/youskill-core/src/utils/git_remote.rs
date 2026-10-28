//! A remote git repository seen through the GitHub REST API: read the branch head, its
//! files, and write a new commit on top of it. The library sync only needs whole-tree
//! commits on one branch, so no local clone (and no libgit2) is involved.

use crate::services::github_auth_service;
use base64::Engine;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::time::Duration;

const API: &str = "https://api.github.com";
const USER_AGENT: &str = "YouSkill";

/// Git file modes the library uses.
pub const MODE_FILE: &str = "100644";
pub const MODE_EXEC: &str = "100755";
pub const MODE_SYMLINK: &str = "120000";

/// One file of a git tree: its mode and blob id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TreeFile {
  pub mode: String,
  pub sha: String,
}

/// Branch head: the commit and its root tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHead {
  pub commit: String,
  pub tree: String,
}

/// Why a head update was not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
  /// The branch moved since it was read: someone else pushed.
  Rejected,
  Other(String),
}

/// The operations the library sync needs from a remote repository.
pub trait RemoteRepo {
  /// Head of the sync branch, `None` for a repository without commits.
  fn head(&self) -> Result<Option<RemoteHead>, String>;
  /// Every file (blob) in a tree, by path.
  fn tree(&self, tree: &str) -> Result<BTreeMap<String, TreeFile>, String>;
  /// Contents of the given files of a commit.
  fn download(
    &self,
    commit: &str,
    files: &BTreeMap<String, TreeFile>,
  ) -> Result<BTreeMap<String, Vec<u8>>, String>;
  fn create_blob(&self, content: &[u8]) -> Result<String, String>;
  /// A tree with exactly these files.
  fn create_tree(&self, files: &BTreeMap<String, TreeFile>) -> Result<String, String>;
  fn create_commit(
    &self,
    message: &str,
    tree: &str,
    parent: Option<&str>,
  ) -> Result<String, String>;
  /// Move the branch to `commit`. Fails with `Rejected` unless it is a fast-forward.
  fn update_head(&self, commit: &str) -> Result<(), UpdateError>;
}

/// Git blob id of some content: `sha1("blob <len>\0" + content)`.
pub fn blob_sha(content: &[u8]) -> String {
  let mut hasher = Sha1::new();
  hasher.update(format!("blob {}\0", content.len()).as_bytes());
  hasher.update(content);
  format!("{:x}", hasher.finalize())
}

/// Repository facts needed to connect.
#[derive(Debug, Clone)]
pub struct RepoInfo {
  pub full_name: String,
  pub private: bool,
  pub can_push: bool,
  pub default_branch: String,
}

pub struct GithubRepo {
  client: Client,
  token: String,
  /// `owner/name`
  repo: String,
  branch: String,
}

#[derive(Deserialize)]
struct RefJson {
  object: ShaJson,
}

#[derive(Deserialize)]
struct ShaJson {
  sha: String,
}

#[derive(Deserialize)]
struct CommitJson {
  tree: ShaJson,
}

#[derive(Deserialize)]
struct TreeJson {
  #[serde(default)]
  tree: Vec<TreeEntryJson>,
  #[serde(default)]
  truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntryJson {
  path: String,
  mode: String,
  #[serde(rename = "type")]
  kind: String,
  sha: String,
}

#[derive(Deserialize)]
struct BlobJson {
  content: String,
}

#[derive(Deserialize)]
struct RepoJson {
  full_name: String,
  private: bool,
  default_branch: Option<String>,
  #[serde(default)]
  permissions: Option<PermissionsJson>,
}

#[derive(Deserialize)]
struct PermissionsJson {
  #[serde(default)]
  push: bool,
}

#[derive(Deserialize)]
struct UserJson {
  login: String,
}

fn client() -> Result<Client, String> {
  Client::builder()
    .timeout(Duration::from_secs(120))
    .build()
    .map_err(|e| e.to_string())
}

/// Token of the GitHub account selected in Settings > GitHub.
pub fn github_token() -> Result<String, String> {
  github_auth_service::token().ok_or_else(|| {
    "GitHub is not connected: install the GitHub CLI and sign in with `gh auth login`".to_string()
  })
}

fn authed(request: RequestBuilder, token: &str) -> RequestBuilder {
  request
    .header("User-Agent", USER_AGENT)
    .header("Accept", "application/vnd.github+json")
    .header("X-GitHub-Api-Version", "2022-11-28")
    .bearer_auth(token)
}

fn send(request: RequestBuilder) -> Result<Response, String> {
  request
    .send()
    .map_err(|e| format!("Could not reach GitHub: {}", e))
}

fn status_error(response: Response, what: &str) -> String {
  let status = response.status();
  let body = response.text().unwrap_or_default();
  let message = serde_json::from_str::<serde_json::Value>(&body)
    .ok()
    .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
    .unwrap_or(body);
  match status {
    StatusCode::UNAUTHORIZED => {
      "GitHub rejected the token of the selected gh account; run `gh auth refresh`".to_string()
    },
    _ => format!("{} failed ({}): {}", what, status.as_u16(), message.trim()),
  }
}

fn json_ok<T: for<'de> Deserialize<'de>>(response: Response, what: &str) -> Result<T, String> {
  if !response.status().is_success() {
    return Err(status_error(response, what));
  }
  response
    .json::<T>()
    .map_err(|e| format!("{}: unexpected response: {}", what, e))
}

/// Login of the account the token belongs to.
pub fn current_login(token: &str) -> Result<String, String> {
  let client = client()?;
  let response = send(authed(client.get(format!("{}/user", API)), token))?;
  Ok(json_ok::<UserJson>(response, "Reading the GitHub user")?.login)
}

/// The repository, or `None` when it does not exist (or the account cannot see it).
pub fn repo_info(token: &str, full_name: &str) -> Result<Option<RepoInfo>, String> {
  let client = client()?;
  let response = send(authed(
    client.get(format!("{}/repos/{}", API, full_name)),
    token,
  ))?;
  if response.status() == StatusCode::NOT_FOUND {
    return Ok(None);
  }
  let repo: RepoJson = json_ok(response, "Reading the repository")?;
  Ok(Some(RepoInfo {
    full_name: repo.full_name,
    private: repo.private,
    can_push: repo.permissions.map(|p| p.push).unwrap_or(false),
    default_branch: repo.default_branch.unwrap_or_else(|| "main".to_string()),
  }))
}

/// Create a private repository with an initial commit, so the branch exists.
pub fn create_private_repo(token: &str, name: &str) -> Result<RepoInfo, String> {
  let client = client()?;
  let response = send(
    authed(client.post(format!("{}/user/repos", API)), token).json(&json!({
      "name": name,
      "private": true,
      "auto_init": true,
      "description": "Skill library backed up by YouSkill",
    })),
  )?;
  let repo: RepoJson = json_ok(response, "Creating the repository")?;
  Ok(RepoInfo {
    full_name: repo.full_name,
    private: repo.private,
    can_push: true,
    default_branch: repo.default_branch.unwrap_or_else(|| "main".to_string()),
  })
}

impl GithubRepo {
  pub fn new(token: String, repo: String, branch: String) -> Result<Self, String> {
    Ok(Self {
      client: client()?,
      token,
      repo,
      branch,
    })
  }

  fn url(&self, path: &str) -> String {
    format!("{}/repos/{}/{}", API, self.repo, path)
  }

  fn get(&self, path: &str) -> Result<Response, String> {
    send(authed(self.client.get(self.url(path)), &self.token))
  }

  /// Give an empty repository its first commit (the Git Data API refuses to write to a
  /// repository without one).
  pub fn initialize(&self) -> Result<(), String> {
    let readme = base64::engine::general_purpose::STANDARD.encode(b"# YouSkill library\n");
    let response = send(
      authed(self.client.put(self.url("contents/README.md")), &self.token).json(&json!({
        "message": "Initialize YouSkill library",
        "content": readme,
        "branch": self.branch,
      })),
    )?;
    if !response.status().is_success() {
      return Err(status_error(response, "Initializing the repository"));
    }
    Ok(())
  }

  fn blob(&self, sha: &str) -> Result<Vec<u8>, String> {
    let blob: BlobJson = json_ok(self.get(&format!("git/blobs/{}", sha))?, "Reading a file")?;
    let compact: String = blob.content.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
      .decode(compact)
      .map_err(|e| format!("Reading a file: {}", e))
  }
}

impl RemoteRepo for GithubRepo {
  fn head(&self) -> Result<Option<RemoteHead>, String> {
    let response = self.get(&format!("git/ref/heads/{}", self.branch))?;
    // 404: no such branch; 409: the repository has no commits.
    if matches!(
      response.status(),
      StatusCode::NOT_FOUND | StatusCode::CONFLICT
    ) {
      return Ok(None);
    }
    let reference: RefJson = json_ok(response, "Reading the branch")?;
    let commit: CommitJson = json_ok(
      self.get(&format!("git/commits/{}", reference.object.sha))?,
      "Reading the commit",
    )?;
    Ok(Some(RemoteHead {
      commit: reference.object.sha,
      tree: commit.tree.sha,
    }))
  }

  fn tree(&self, tree: &str) -> Result<BTreeMap<String, TreeFile>, String> {
    let json: TreeJson = json_ok(
      self.get(&format!("git/trees/{}?recursive=1", tree))?,
      "Reading the file list",
    )?;
    if json.truncated {
      return Err("The repository has too many files to read in one request".to_string());
    }
    Ok(
      json
        .tree
        .into_iter()
        .filter(|entry| entry.kind == "blob")
        .map(|entry| {
          (
            entry.path,
            TreeFile {
              mode: entry.mode,
              sha: entry.sha,
            },
          )
        })
        .collect(),
    )
  }

  /// One zipball for the whole commit; a file whose content does not match its blob id
  /// (a symlink, an archive quirk) is read on its own.
  fn download(
    &self,
    commit: &str,
    files: &BTreeMap<String, TreeFile>,
  ) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let response = self.get(&format!("zipball/{}", commit))?;
    if !response.status().is_success() {
      return Err(status_error(response, "Downloading the library"));
    }
    let bytes = response
      .bytes()
      .map_err(|e| format!("Downloading the library: {}", e))?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
      .map_err(|e| format!("Reading the library archive: {}", e))?;
    let mut archived: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for index in 0..archive.len() {
      let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
      if entry.is_dir() {
        continue;
      }
      // Entries are prefixed with `<owner>-<repo>-<sha>/`.
      let name = entry.name().to_string();
      let Some((_, path)) = name.split_once('/') else {
        continue;
      };
      if !files.contains_key(path) {
        continue;
      }
      let mut content = Vec::new();
      entry.read_to_end(&mut content).map_err(|e| e.to_string())?;
      archived.insert(path.to_string(), content);
    }

    let mut result = BTreeMap::new();
    for (path, file) in files {
      let content = match archived.remove(path) {
        Some(content) if blob_sha(&content) == file.sha => content,
        _ => self.blob(&file.sha)?,
      };
      if blob_sha(&content) != file.sha {
        return Err(format!("Downloaded content of {} does not match", path));
      }
      result.insert(path.clone(), content);
    }
    Ok(result)
  }

  fn create_blob(&self, content: &[u8]) -> Result<String, String> {
    let response = send(
      authed(self.client.post(self.url("git/blobs")), &self.token).json(&json!({
        "content": base64::engine::general_purpose::STANDARD.encode(content),
        "encoding": "base64",
      })),
    )?;
    Ok(json_ok::<ShaJson>(response, "Uploading a file")?.sha)
  }

  fn create_tree(&self, files: &BTreeMap<String, TreeFile>) -> Result<String, String> {
    let entries: Vec<serde_json::Value> = files
      .iter()
      .map(
        |(path, file)| json!({ "path": path, "mode": file.mode, "type": "blob", "sha": file.sha }),
      )
      .collect();
    let response = send(
      authed(self.client.post(self.url("git/trees")), &self.token)
        .json(&json!({ "tree": entries })),
    )?;
    Ok(json_ok::<ShaJson>(response, "Writing the file list")?.sha)
  }

  fn create_commit(
    &self,
    message: &str,
    tree: &str,
    parent: Option<&str>,
  ) -> Result<String, String> {
    let parents: Vec<&str> = parent.into_iter().collect();
    let response = send(
      authed(self.client.post(self.url("git/commits")), &self.token).json(&json!({
        "message": message,
        "tree": tree,
        "parents": parents,
      })),
    )?;
    Ok(json_ok::<ShaJson>(response, "Writing the commit")?.sha)
  }

  fn update_head(&self, commit: &str) -> Result<(), UpdateError> {
    let response = send(
      authed(
        self
          .client
          .patch(self.url(&format!("git/refs/heads/{}", self.branch))),
        &self.token,
      )
      .json(&json!({ "sha": commit, "force": false })),
    )
    .map_err(UpdateError::Other)?;
    match response.status() {
      status if status.is_success() => Ok(()),
      StatusCode::UNPROCESSABLE_ENTITY | StatusCode::CONFLICT => Err(UpdateError::Rejected),
      _ => Err(UpdateError::Other(status_error(
        response,
        "Moving the branch",
      ))),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn blob_sha_matches_git() {
    // `printf 'hello\n' | git hash-object --stdin`
    assert_eq!(
      blob_sha(b"hello\n"),
      "ce013625030ba8dba906f756967f9e9ca394464a"
    );
    assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
  }
}
