//! GitHub Releases API registry implementation

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::warn;

/// Default base URL for GitHub API
const DEFAULT_BASE_URL: &str = "https://api.github.com";

/// Response from GitHub Releases API
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    published_at: Option<String>,
}

/// Response from GitHub Tags API
#[derive(Debug, Deserialize)]
struct Tag {
    name: String,
    commit: TagCommit,
}

/// Commit information from GitHub Tags API
#[derive(Debug, Deserialize)]
struct TagCommit {
    sha: String,
}

/// Trait for fetching commit SHA for a specific tag
#[async_trait::async_trait]
pub trait TagShaFetcher: Send + Sync {
    /// Fetch the commit SHA for a specific tag
    async fn fetch_tag_sha(
        &self,
        package_name: &str,
        tag_name: &str,
    ) -> Result<String, RegistryError>;
}

/// Registry implementation for GitHub Releases API
pub struct GitHubRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl GitHubRegistry {
    /// Creates a new GitHubRegistry with a custom base URL
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("version-lsp")
                .build()
                .expect("Failed to create HTTP client"),
            base_url: base_url.to_string(),
        }
    }
}

impl Default for GitHubRegistry {
    fn default() -> Self {
        // Allow overriding base URL for testing
        let base_url =
            std::env::var("GITHUB_API_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self::new(&base_url)
    }
}

#[async_trait::async_trait]
impl Registry for GitHubRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::GitHubActions
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        let url = format!("{}/repos/{}/releases", self.base_url, package_name);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            return Err(RegistryError::RateLimited {
                retry_after_secs: retry_after,
            });
        }

        if !status.is_success() {
            warn!("GitHub API returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let releases: Vec<Release> = response.json().await.map_err(|e| {
            warn!("Failed to parse GitHub releases response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Sort releases by published_at (oldest first, newest last)
        // Releases without published_at are placed at the beginning
        let mut releases_with_dates: Vec<(String, Option<DateTime<Utc>>)> = releases
            .into_iter()
            .map(|r| {
                let timestamp = r
                    .published_at
                    .and_then(|ts| DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                (r.tag_name, timestamp)
            })
            .collect();

        releases_with_dates.sort_by(|(_, a), (_, b)| a.cmp(b));

        let versions = releases_with_dates
            .into_iter()
            .map(|(tag, _)| tag)
            .collect();

        Ok(PackageVersions::new(versions))
    }
}

#[async_trait::async_trait]
impl TagShaFetcher for GitHubRegistry {
    async fn fetch_tag_sha(
        &self,
        package_name: &str,
        tag_name: &str,
    ) -> Result<String, RegistryError> {
        let url = format!("{}/repos/{}/tags", self.base_url, package_name);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok());
            return Err(RegistryError::RateLimited {
                retry_after_secs: retry_after,
            });
        }

        if !status.is_success() {
            warn!("GitHub API returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let tags: Vec<Tag> = response.json().await.map_err(|e| {
            warn!("Failed to parse GitHub tags response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Find the tag with matching name
        tags.into_iter()
            .find(|t| t.name == tag_name)
            .map(|t| t.commit.sha)
            .ok_or_else(|| RegistryError::NotFound(format!("Tag {} not found", tag_name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_releases_sorted_by_published_at() {
        let mut server = Server::new_async().await;

        // GitHub API returns releases in descending order (newest first)
        // But we need them sorted oldest first, newest last
        let mock = server
            .mock("GET", "/repos/actions/checkout/releases")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"tag_name": "v4.1.0", "published_at": "2024-01-15T00:00:00Z"},
                    {"tag_name": "v4.0.0", "published_at": "2024-01-01T00:00:00Z"},
                    {"tag_name": "v3.6.0", "published_at": "2023-12-01T00:00:00Z"}
                ]"#,
            )
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry
            .fetch_all_versions("actions/checkout")
            .await
            .unwrap();

        mock.assert_async().await;
        // Should be sorted oldest first, newest last
        assert_eq!(
            result.versions,
            vec![
                "v3.6.0".to_string(),
                "v4.0.0".to_string(),
                "v4.1.0".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_repo() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/nonexistent/repo/releases")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"message": "Not Found"}"#)
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry.fetch_all_versions("nonexistent/repo").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_rate_limited_for_429() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/actions/checkout/releases")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_header("retry-after", "60")
            .with_body(r#"{"message": "API rate limit exceeded"}"#)
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry.fetch_all_versions("actions/checkout").await;

        mock.assert_async().await;
        assert!(matches!(
            result,
            Err(RegistryError::RateLimited {
                retry_after_secs: Some(60)
            })
        ));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_empty_for_repo_without_releases() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/some/repo/releases")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("[]")
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry.fetch_all_versions("some/repo").await.unwrap();

        mock.assert_async().await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_tag_sha_returns_sha_for_existing_tag() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/actions/checkout/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"name": "v4.1.6", "commit": {"sha": "8e5e7e5ab8b370d6c329ec480221332ada57f0ab"}},
                    {"name": "v4.1.5", "commit": {"sha": "abcdef1234567890abcdef1234567890abcdef12"}}
                ]"#,
            )
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry
            .fetch_tag_sha("actions/checkout", "v4.1.6")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(result, "8e5e7e5ab8b370d6c329ec480221332ada57f0ab");
    }

    #[tokio::test]
    async fn fetch_tag_sha_returns_not_found_for_nonexistent_tag() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/actions/checkout/tags")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"[
                    {"name": "v4.1.5", "commit": {"sha": "abcdef1234567890abcdef1234567890abcdef12"}}
                ]"#,
            )
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry.fetch_tag_sha("actions/checkout", "v4.1.6").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_tag_sha_returns_rate_limited_for_429() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/repos/actions/checkout/tags")
            .with_status(429)
            .with_header("content-type", "application/json")
            .with_header("retry-after", "120")
            .with_body(r#"{"message": "API rate limit exceeded"}"#)
            .create_async()
            .await;

        let registry = GitHubRegistry::new(&server.url());
        let result = registry.fetch_tag_sha("actions/checkout", "v4.1.6").await;

        mock.assert_async().await;
        assert!(matches!(
            result,
            Err(RegistryError::RateLimited {
                retry_after_secs: Some(120)
            })
        ));
    }
}
