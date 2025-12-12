//! GitHub Releases API registry implementation

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use serde::Deserialize;
use tracing::warn;

/// Default base URL for GitHub API
const DEFAULT_BASE_URL: &str = "https://api.github.com";

/// Response from GitHub Releases API
#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
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
        Self::new(DEFAULT_BASE_URL)
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

        let versions = releases.into_iter().map(|r| r.tag_name).collect();

        Ok(PackageVersions::new(versions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_releases_sorted_by_newest() {
        let mut server = Server::new_async().await;

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
        assert_eq!(
            result.versions,
            vec![
                "v4.1.0".to_string(),
                "v4.0.0".to_string(),
                "v3.6.0".to_string()
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
}
