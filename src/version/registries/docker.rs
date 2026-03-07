//! Docker Registry HTTP API V2 client
//!
//! Supports fetching tags from Docker Hub and ghcr.io.
//! Dispatches to the appropriate registry based on the package name prefix.

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use semver::Version;
use serde::Deserialize;
use tracing::warn;

/// Docker Hub registry URL
const DOCKER_HUB_REGISTRY_URL: &str = "https://registry-1.docker.io";
/// Docker Hub auth URL
const DOCKER_HUB_AUTH_URL: &str = "https://auth.docker.io/token";
/// Docker Hub service name
const DOCKER_HUB_SERVICE: &str = "registry.docker.io";

/// ghcr.io registry URL
const GHCR_REGISTRY_URL: &str = "https://ghcr.io";
/// ghcr.io auth URL
const GHCR_AUTH_URL: &str = "https://ghcr.io/token";
/// ghcr.io service name
const GHCR_SERVICE: &str = "ghcr.io";

/// Token response from the auth endpoint
#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
}

/// Tag list response from the registry
#[derive(Debug, Deserialize)]
struct TagListResponse {
    tags: Option<Vec<String>>,
}

/// Registry implementation for Docker (Docker Hub + ghcr.io)
pub struct DockerRegistry {
    client: reqwest::Client,
    /// Override base URLs for testing
    docker_hub_registry_url: String,
    docker_hub_auth_url: String,
    ghcr_registry_url: String,
    ghcr_auth_url: String,
}

impl DockerRegistry {
    /// Create a new DockerRegistry with custom URLs (for testing)
    pub fn new(
        docker_hub_registry_url: &str,
        docker_hub_auth_url: &str,
        ghcr_registry_url: &str,
        ghcr_auth_url: &str,
    ) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("version-lsp")
                .build()
                .expect("Failed to create HTTP client"),
            docker_hub_registry_url: docker_hub_registry_url.to_string(),
            docker_hub_auth_url: docker_hub_auth_url.to_string(),
            ghcr_registry_url: ghcr_registry_url.to_string(),
            ghcr_auth_url: ghcr_auth_url.to_string(),
        }
    }

    /// Fetch a token for the given repository
    async fn fetch_token(
        &self,
        auth_url: &str,
        service: &str,
        repository: &str,
    ) -> Result<String, RegistryError> {
        let url = format!(
            "{}?service={}&scope=repository:{}:pull",
            auth_url, service, repository
        );

        let response = self.client.get(&url).send().await?;
        let status = response.status();

        if !status.is_success() {
            warn!("Docker auth failed with status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Auth failed: {}",
                status
            )));
        }

        let token_resp: TokenResponse = response.json().await.map_err(|e| {
            warn!("Failed to parse token response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        Ok(token_resp.token)
    }

    /// Fetch tag list for the given repository
    async fn fetch_tags(
        &self,
        registry_url: &str,
        repository: &str,
        token: &str,
    ) -> Result<Vec<String>, RegistryError> {
        let url = format!("{}/v2/{}/tags/list", registry_url, repository);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(repository.to_string()));
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
            warn!("Docker registry returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let tag_list: TagListResponse = response.json().await.map_err(|e| {
            warn!("Failed to parse tag list response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        Ok(tag_list.tags.unwrap_or_default())
    }

    /// Resolve which registry config and API repository name to use
    fn resolve_registry(&self, package_name: &str) -> (&str, &str, &str, String) {
        if let Some(repo) = package_name.strip_prefix("ghcr.io/") {
            (
                &self.ghcr_auth_url,
                &self.ghcr_registry_url,
                GHCR_SERVICE,
                repo.to_string(),
            )
        } else {
            (
                &self.docker_hub_auth_url,
                &self.docker_hub_registry_url,
                DOCKER_HUB_SERVICE,
                package_name.to_string(),
            )
        }
    }
}

impl Default for DockerRegistry {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("version-lsp")
                .build()
                .expect("Failed to create HTTP client"),
            docker_hub_registry_url: DOCKER_HUB_REGISTRY_URL.to_string(),
            docker_hub_auth_url: DOCKER_HUB_AUTH_URL.to_string(),
            ghcr_registry_url: GHCR_REGISTRY_URL.to_string(),
            ghcr_auth_url: GHCR_AUTH_URL.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Registry for DockerRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Docker
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        let (auth_url, registry_url, service, repository) = self.resolve_registry(package_name);

        // Step 1: Get token
        let token = self.fetch_token(auth_url, service, &repository).await?;

        // Step 2: Fetch tags
        let tags = self.fetch_tags(registry_url, &repository, &token).await?;

        // Step 3: Filter and sort tags
        let versions = filter_and_sort_tags(tags);

        Ok(PackageVersions::new(versions))
    }
}

/// Filter tags to only include version-like tags and sort them.
///
/// - Only keeps tags starting with a digit (filters out "latest", "alpine", "stable", etc.)
/// - Sorts by semver version (ascending: oldest first, newest last)
/// - Within the same version, suffixless tags come last
fn filter_and_sort_tags(tags: Vec<String>) -> Vec<String> {
    let mut versioned: Vec<(String, Version, String)> = tags
        .into_iter()
        .filter_map(|tag| {
            // Strip v/V prefix for initial digit check
            let stripped = tag
                .strip_prefix('v')
                .or_else(|| tag.strip_prefix('V'))
                .unwrap_or(&tag);

            // Must start with a digit
            if !stripped.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }

            // Extract version part (digits and dots)
            let version_end = stripped
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(stripped.len());

            let version_part = &stripped[..version_end];
            let suffix = stripped[version_end..].to_string();

            // Normalize to semver
            let parts: Vec<&str> = version_part.split('.').collect();
            let normalized = match parts.len() {
                1 => format!("{}.0.0", parts[0]),
                2 => format!("{}.{}.0", parts[0], parts[1]),
                3 => format!("{}.{}.{}", parts[0], parts[1], parts[2]),
                _ => return None,
            };

            let semver = Version::parse(&normalized).ok()?;
            Some((tag, semver, suffix))
        })
        .collect();

    // Sort by version ascending, then suffixless last within same version
    versioned.sort_by(|(_, ver_a, suffix_a), (_, ver_b, suffix_b)| {
        ver_a.cmp(ver_b).then_with(|| {
            // Within same version: suffixed before suffixless
            match (suffix_a.is_empty(), suffix_b.is_empty()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => suffix_a.cmp(suffix_b),
            }
        })
    });

    versioned.into_iter().map(|(tag, _, _)| tag).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use rstest::rstest;

    #[tokio::test]
    async fn fetch_all_versions_returns_filtered_sorted_tags_for_docker_hub() {
        let mut auth_server = Server::new_async().await;
        let mut registry_server = Server::new_async().await;

        let auth_mock = auth_server
            .mock("GET", "/token")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("service".into(), "registry.docker.io".into()),
                mockito::Matcher::UrlEncoded(
                    "scope".into(),
                    "repository:library/nginx:pull".into(),
                ),
            ]))
            .with_status(200)
            .with_body(r#"{"token": "test-token"}"#)
            .create_async()
            .await;

        let tags_mock = registry_server
            .mock("GET", "/v2/library/nginx/tags/list")
            .match_header("authorization", "Bearer test-token")
            .with_status(200)
            .with_body(r#"{"tags": ["latest", "1.25", "1.25-alpine", "1.27", "1.27-alpine", "alpine", "stable"]}"#)
            .create_async()
            .await;

        let auth_url = format!("{}/token", auth_server.url());
        let registry = DockerRegistry::new(&registry_server.url(), &auth_url, "", "");

        let result = registry.fetch_all_versions("library/nginx").await.unwrap();

        auth_mock.assert_async().await;
        tags_mock.assert_async().await;

        // Should only include versioned tags, sorted ascending
        assert_eq!(
            result.versions,
            vec![
                "1.25-alpine".to_string(),
                "1.25".to_string(),
                "1.27-alpine".to_string(),
                "1.27".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_image() {
        let mut auth_server = Server::new_async().await;
        let mut registry_server = Server::new_async().await;

        let auth_mock = auth_server
            .mock("GET", "/token")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"token": "test-token"}"#)
            .create_async()
            .await;

        let tags_mock = registry_server
            .mock("GET", "/v2/library/nonexistent/tags/list")
            .with_status(404)
            .create_async()
            .await;

        let auth_url = format!("{}/token", auth_server.url());
        let registry = DockerRegistry::new(&registry_server.url(), &auth_url, "", "");

        let result = registry.fetch_all_versions("library/nonexistent").await;

        auth_mock.assert_async().await;
        tags_mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_rate_limited_for_429() {
        let mut auth_server = Server::new_async().await;
        let mut registry_server = Server::new_async().await;

        let auth_mock = auth_server
            .mock("GET", "/token")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_body(r#"{"token": "test-token"}"#)
            .create_async()
            .await;

        let tags_mock = registry_server
            .mock("GET", "/v2/library/nginx/tags/list")
            .with_status(429)
            .with_header("retry-after", "60")
            .create_async()
            .await;

        let auth_url = format!("{}/token", auth_server.url());
        let registry = DockerRegistry::new(&registry_server.url(), &auth_url, "", "");

        let result = registry.fetch_all_versions("library/nginx").await;

        auth_mock.assert_async().await;
        tags_mock.assert_async().await;
        assert!(matches!(
            result,
            Err(RegistryError::RateLimited {
                retry_after_secs: Some(60)
            })
        ));
    }

    #[tokio::test]
    async fn fetch_all_versions_dispatches_ghcr_correctly() {
        let mut ghcr_auth_server = Server::new_async().await;
        let mut ghcr_registry_server = Server::new_async().await;

        let auth_mock = ghcr_auth_server
            .mock("GET", "/token")
            .match_query(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("service".into(), "ghcr.io".into()),
                mockito::Matcher::UrlEncoded("scope".into(), "repository:owner/repo:pull".into()),
            ]))
            .with_status(200)
            .with_body(r#"{"token": "ghcr-token"}"#)
            .create_async()
            .await;

        let tags_mock = ghcr_registry_server
            .mock("GET", "/v2/owner/repo/tags/list")
            .match_header("authorization", "Bearer ghcr-token")
            .with_status(200)
            .with_body(r#"{"tags": ["v1.0.0", "v2.0.0", "latest"]}"#)
            .create_async()
            .await;

        let ghcr_auth_url = format!("{}/token", ghcr_auth_server.url());
        let registry = DockerRegistry::new("", "", &ghcr_registry_server.url(), &ghcr_auth_url);

        let result = registry
            .fetch_all_versions("ghcr.io/owner/repo")
            .await
            .unwrap();

        auth_mock.assert_async().await;
        tags_mock.assert_async().await;

        assert_eq!(
            result.versions,
            vec!["v1.0.0".to_string(), "v2.0.0".to_string()]
        );
    }

    #[rstest]
    #[case(
        vec!["latest", "1.25", "1.25-alpine", "1.27", "1.27-alpine", "alpine"],
        vec!["1.25-alpine", "1.25", "1.27-alpine", "1.27"]
    )]
    #[case(
        vec!["v1.0.0", "v2.0.0", "latest", "stable"],
        vec!["v1.0.0", "v2.0.0"]
    )]
    #[case(
        vec!["15", "16", "17", "latest", "alpine"],
        vec!["15", "16", "17"]
    )]
    #[case(
        vec![],
        vec![]
    )]
    fn filter_and_sort_tags_returns_expected(
        #[case] input: Vec<&str>,
        #[case] expected: Vec<&str>,
    ) {
        let tags: Vec<String> = input.into_iter().map(|s| s.to_string()).collect();
        let result = filter_and_sort_tags(tags);
        let expected: Vec<String> = expected.into_iter().map(|s| s.to_string()).collect();
        assert_eq!(result, expected);
    }
}
