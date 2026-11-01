use crate::models::{RemoteSkill, RemoteSkillsResponse};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub(crate) const API_KEY: &str = "4586a151c7950fa482a099dfa916ff9d616d02d9f10e710c31819607aabb9fff";
pub(crate) const API_BASE_URL: &str = "https://api.youskill.cc";

#[derive(Debug, Deserialize)]
struct SkillsResponse {
  skills: Vec<ApiSkill>,
  total: i64,
}

#[derive(Debug, Deserialize)]
struct ApiSkill {
  id: i64,
  name: String,
  source: String,
  url: String,
  #[allow(dead_code)]
  description: Option<String>,
  #[serde(default)]
  star_count: i64,
  #[serde(default)]
  heat_score: i64,
  #[serde(default)]
  install_count: i64,
  path: Option<String>,
  /// GitHub tree SHA for skill folder (for update detection)
  #[serde(default)]
  skill_path_sha: Option<String>,
  /// Git branch name (e.g., "main", "master")
  #[serde(default)]
  branch: Option<String>,
}

pub async fn fetch_remote_skills(
  skip: Option<u32>,
  limit: Option<u32>,
  search: Option<String>,
  sort_by: Option<String>,
  sort_order: Option<String>,
  tag_id: Option<u64>,
  source: Option<String>,
) -> Result<RemoteSkillsResponse, String> {
  let client = Client::builder()
    .timeout(Duration::from_secs(10))
    .build()
    .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
  let base_url = format!("{}/api/public/skills", API_BASE_URL);

  // Sorting params - default to heat_score desc
  let sort_by = sort_by.unwrap_or_else(|| "heat_score".to_string());
  let sort_order = sort_order.unwrap_or_else(|| "desc".to_string());

  let response: SkillsResponse = match client
    .get(base_url)
    .query(
      &[
        ("skip", skip.map(|s| s.to_string())),
        ("limit", limit.map(|l| l.to_string())),
        (
          "search",
          search
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string()),
        ),
        ("tag_id", tag_id.map(|id| id.to_string())),
        ("source", source),
        ("sort_by", Some(sort_by)),
        ("sort_order", Some(sort_order)),
      ]
      .into_iter()
      .filter_map(|(k, v)| v.map(|val| (k, val)))
      .collect::<Vec<_>>(),
    )
    .send()
    .await
  {
    Ok(resp) => {
      if resp.status().is_success() {
        resp
          .json::<SkillsResponse>()
          .await
          .map_err(|e| format!("JSON 解析失败: {}", e))?
      } else {
        return Err(format!("API 返回错误: {}", resp.status()));
      }
    },
    Err(e) => {
      return Err(format!("网络连接失败: {}", e));
    },
  };

  let total = response.total;
  let skills: Vec<RemoteSkill> = response.skills.into_iter().map(RemoteSkill::from).collect();
  let has_more = has_more_results(skip.unwrap_or(0), skills.len(), total);

  Ok(RemoteSkillsResponse {
    skills,
    total,
    has_more,
  })
}

/// Fetch skills by a list of names (for update detection)
pub async fn fetch_skills_by_names(names: Vec<String>) -> Result<Vec<RemoteSkill>, String> {
  tracing::info!("Fetching skills by names: {:?}", names);
  if names.is_empty() {
    return Ok(Vec::new());
  }

  let client = Client::builder()
    .timeout(Duration::from_secs(10))
    .build()
    .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

  let url = format!("{}/api/skills/by-names", API_BASE_URL);
  let names_param = names.join(",");

  let response: Vec<ApiSkill> = match client
    .get(url)
    .query(&[("names", names_param)])
    .header("X-API-Key", API_KEY)
    .send()
    .await
  {
    Ok(resp) => {
      if resp.status().is_success() {
        resp
          .json::<Vec<ApiSkill>>()
          .await
          .map_err(|e| format!("JSON parsing failed: {}", e))?
      } else {
        return Err(format!("API returned error: {}", resp.status()));
      }
    },
    Err(e) => {
      return Err(format!("Network connection failed: {}", e));
    },
  };

  let skills = response.into_iter().map(RemoteSkill::from).collect();

  Ok(skills)
}

pub async fn record_skill_install(skill_id: String) -> Result<(), String> {
  let client = Client::builder()
    .timeout(Duration::from_secs(10))
    .build()
    .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

  let url = format!("{}/api/skills/{}/install", API_BASE_URL, skill_id);

  let response = client.post(&url).header("X-API-Key", API_KEY).send().await;

  match response {
    Ok(resp) => {
      if resp.status().is_success() {
        Ok(())
      } else {
        Err(format!("API returned error: {}", resp.status()))
      }
    },
    Err(e) => Err(format!("Failed to record install: {}", e)),
  }
}

fn has_more_results(skip: u32, count: usize, total: i64) -> bool {
  count > 0 && i64::from(skip) + (count as i64) < total
}

impl From<ApiSkill> for RemoteSkill {
  fn from(skill: ApiSkill) -> Self {
    Self {
      id: skill.id.to_string(),
      skill_id: skill.id.to_string(),
      name: skill.name,
      description: skill.description.filter(|s| !s.trim().is_empty()),
      star_count: skill.star_count.max(0) as u64,
      heat_score: skill.heat_score.max(0) as u64,
      install_count: skill.install_count.max(0) as u64,
      source: skill.source,
      url: Some(skill.url).filter(|s| !s.is_empty()),
      path: skill.path.filter(|s| !s.is_empty()),
      skill_path_sha: skill.skill_path_sha.filter(|s| !s.is_empty()),
      branch: skill.branch.filter(|s| !s.is_empty()),
    }
  }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MarketCategory {
  pub icon: Option<String>,
  pub id: u64,
  pub name: String,
  pub skill_count: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MarketRepository {
  pub star_count: Option<u64>,
  pub icon: Option<String>,
  pub id: u64,
  pub name: String,
  pub skill_count: u64,
  pub enabled: bool,
  #[serde(rename = "type")]
  pub repo_type: String,
}

#[derive(Deserialize)]
struct CategoriesResponse {
  tags: Vec<MarketCategory>,
}

#[derive(Deserialize)]
struct RepositoriesResponse {
  items: Vec<MarketRepository>,
}

async fn fetch_catalog_from<T: serde::de::DeserializeOwned>(
  client: &Client,
  base_url: &str,
  path: &str,
) -> Result<T, reqwest::Error> {
  client
    .get(format!("{base_url}/api/public/{path}"))
    .send()
    .await?
    .error_for_status()?
    .json::<T>()
    .await
}

fn catalog_client() -> Result<Client, String> {
  Client::builder()
    .timeout(Duration::from_secs(10))
    .build()
    .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

async fn fetch_catalog<T: serde::de::DeserializeOwned>(path: &str) -> Result<T, String> {
  fetch_catalog_from(&catalog_client()?, API_BASE_URL, path)
    .await
    .map_err(|e| format!("Catalog request failed: {e}"))
}

async fn fetch_market_categories_from(
  client: &Client,
  base_url: &str,
) -> Result<Vec<MarketCategory>, reqwest::Error> {
  let response =
    match fetch_catalog_from::<CategoriesResponse>(client, base_url, "tags/summary").await {
      Ok(response) => response,
      // Older servers parse "summary" as a numeric tag id and return 422.
      Err(error)
        if matches!(
          error.status().map(|status| status.as_u16()),
          Some(404 | 422)
        ) =>
      {
        client
        .get(format!("{base_url}/api/public/tags"))
        // The legacy response includes all related skills and can be large.
        .timeout(Duration::from_secs(45))
        .send()
        .await?
        .error_for_status()?
        .json::<CategoriesResponse>()
        .await?
      },
      Err(error) => return Err(error),
    };
  Ok(response.tags)
}

pub async fn fetch_market_categories() -> Result<Vec<MarketCategory>, String> {
  fetch_market_categories_from(&catalog_client()?, API_BASE_URL)
    .await
    .map_err(|e| format!("Catalog request failed: {e}"))
}

pub async fn fetch_hot_repositories() -> Result<Vec<MarketRepository>, String> {
  Ok(
    fetch_catalog::<RepositoriesResponse>("featured-repos")
      .await?
      .items
      .into_iter()
      .filter(|repo| repo.enabled && repo.repo_type == "hot")
      .collect(),
  )
}

pub async fn fetch_remote_skill(skill_id: String) -> Result<RemoteSkill, String> {
  let id: u64 = skill_id
    .parse()
    .map_err(|_| "Invalid marketplace skill id")?;
  Ok(
    fetch_catalog::<ApiSkill>(&format!("skills/{id}"))
      .await?
      .into(),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn catalog_server(
    responses: Vec<(u16, &'static str)>,
  ) -> (String, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
      let mut paths = Vec::new();
      for (status, body) in responses {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
          match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
              assert!(
                std::time::Instant::now() < deadline,
                "Expected another catalog request"
              );
              std::thread::sleep(Duration::from_millis(5));
            },
            Err(error) => panic!("Failed to accept request: {error}"),
          }
        };
        stream.set_nonblocking(false).unwrap();
        stream
          .set_read_timeout(Some(Duration::from_secs(5)))
          .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 1024];
        while !request.windows(4).any(|value| value == b"\r\n\r\n") {
          let count = stream.read(&mut buffer).unwrap();
          assert!(count > 0, "Incomplete HTTP request");
          request.extend_from_slice(&buffer[..count]);
        }
        let request = String::from_utf8(request).unwrap();
        paths.push(
          request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string(),
        );
        write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
      }
      paths
    });
    (format!("http://{address}"), server)
  }

  fn categories_from(base: &str) -> Result<Vec<MarketCategory>, reqwest::Error> {
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
      .unwrap()
      .block_on(fetch_market_categories_from(&Client::new(), base))
  }

  const CATEGORY_JSON: &str = r#"{"tags":[{"id":7,"name":"testing","skill_count":2,"skills":[]}]}"#;

  #[test]
  fn catalog_icons_survive_the_frontend_boundary() {
    let category: MarketCategory = serde_json::from_value(serde_json::json!({
      "id": 7, "name": "testing", "skill_count": 2,
      "icon": "streamline-freehand:bug-alert-message"
    }))
    .unwrap();
    assert_eq!(
      serde_json::to_value(category).unwrap()["icon"],
      "streamline-freehand:bug-alert-message"
    );
    let repository: MarketRepository = serde_json::from_value(serde_json::json!({
      "id": 1, "name": "team/tools", "skill_count": 2, "enabled": true,
      "type": "hot", "icon": "lucide:star", "star_count": 120
    }))
    .unwrap();
    assert_eq!(repository.star_count, Some(120));
    assert_eq!(
      serde_json::to_value(repository).unwrap()["icon"],
      "lucide:star"
    );
  }

  #[test]
  fn categories_prefer_the_summary_endpoint() {
    let (base, server) = catalog_server(vec![(200, CATEGORY_JSON)]);
    let categories = categories_from(&base).unwrap();
    assert_eq!(categories[0].id, 7);
    assert_eq!(server.join().unwrap(), ["/api/public/tags/summary"]);
  }

  #[test]
  fn categories_support_servers_without_the_summary_route() {
    for status in [404, 422] {
      let (base, server) = catalog_server(vec![(status, "{}"), (200, CATEGORY_JSON)]);
      let categories = categories_from(&base).unwrap();
      assert_eq!(categories[0].name, "testing");
      assert_eq!(categories[0].skill_count, 2);
      assert_eq!(
        server.join().unwrap(),
        ["/api/public/tags/summary", "/api/public/tags"]
      );
    }
  }

  #[test]
  fn categories_do_not_retry_authorization_rate_limit_or_server_errors() {
    for status in [401, 403, 429, 500] {
      let (base, server) = catalog_server(vec![(status, "{}")]);
      let error = categories_from(&base).unwrap_err();
      assert_eq!(error.status().map(|value| value.as_u16()), Some(status));
      assert_eq!(server.join().unwrap(), ["/api/public/tags/summary"]);
    }
  }

  #[test]
  fn categories_do_not_hide_invalid_summary_responses() {
    let (base, server) = catalog_server(vec![(200, "{}")]);
    let error = categories_from(&base).unwrap_err();
    assert!(error.is_decode());
    assert_eq!(server.join().unwrap(), ["/api/public/tags/summary"]);
  }

  #[test]
  fn pagination_uses_offset_instead_of_one_based_page() {
    assert!(has_more_results(0, 20, 40));
    assert!(!has_more_results(20, 20, 40));
    assert!(has_more_results(20, 20, 41));
    assert!(!has_more_results(40, 1, 41));
    assert!(!has_more_results(0, 0, 40));
  }

  #[test]
  fn skill_mapping_preserves_identity_and_source() {
    let skill: ApiSkill = serde_json::from_str(
      r#"{
      "id": 42, "name": "review", "source": "team/tools", "url": "https://example.com",
      "description": " ", "path": "skills/review", "branch": "main"
    }"#,
    )
    .unwrap();
    let remote = RemoteSkill::from(skill);
    assert_eq!(remote.id, "42");
    assert_eq!(remote.source, "team/tools");
    assert_eq!(remote.path.as_deref(), Some("skills/review"));
    assert_eq!(remote.branch.as_deref(), Some("main"));
    assert_eq!(remote.description, None);
  }
}
