//! crates.io registry API implementation

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::warn;

/// Default base URL for crates.io registry
const DEFAULT_BASE_URL: &str = "https://crates.io/api/v1/crates";

/// Response from crates.io registry API
#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    versions: Vec<CrateVersion>,
}

/// Version information from crates.io
#[derive(Debug, Deserialize)]
struct CrateVersion {
    num: String,
    yanked: bool,
    created_at: String,
}

/// Registry implementation for crates.io API
pub struct CratesIoRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl CratesIoRegistry {
    /// Creates a new CratesIoRegistry with a custom base URL
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

impl Default for CratesIoRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

#[async_trait::async_trait]
impl Registry for CratesIoRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::CratesIo
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        let url = format!("{}/{}", self.base_url, package_name);

        let response = self.client.get(&url).send().await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if !status.is_success() {
            warn!("crates.io registry returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let crate_info: CratesIoResponse = response.json().await.map_err(|e| {
            warn!("Failed to parse crates.io registry response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Filter out yanked versions and sort by created_at (oldest first, newest last)
        let mut versions: Vec<(String, Option<DateTime<Utc>>)> = crate_info
            .versions
            .into_iter()
            .filter(|v| !v.yanked)
            .map(|v| {
                let timestamp = DateTime::parse_from_rfc3339(&v.created_at)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc));
                (v.num, timestamp)
            })
            .collect();

        versions.sort_by_key(|(_, a)| *a);

        let versions: Vec<String> = versions.into_iter().map(|(v, _)| v).collect();

        Ok(PackageVersions::new(versions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_sorted_by_created_at() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/serde")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "crate": {
                        "id": "serde",
                        "name": "serde"
                    },
                    "versions": [
                        {"num": "1.0.2", "yanked": false, "created_at": "2020-03-01T00:00:00.000Z"},
                        {"num": "1.0.0", "yanked": false, "created_at": "2020-01-01T00:00:00.000Z"},
                        {"num": "1.0.1", "yanked": false, "created_at": "2020-02-01T00:00:00.000Z"}
                    ]
                }"#,
            )
            .create_async()
            .await;

        let registry = CratesIoRegistry::new(&server.url());
        let result = registry.fetch_all_versions("serde").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by created_at (oldest first, newest last)
        assert_eq!(
            result.versions,
            vec![
                "1.0.0".to_string(),
                "1.0.1".to_string(),
                "1.0.2".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_crate() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/nonexistent-crate")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"errors": [{"detail": "Not Found"}]}"#)
            .create_async()
            .await;

        let registry = CratesIoRegistry::new(&server.url());
        let result = registry.fetch_all_versions("nonexistent-crate").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_excludes_yanked_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/test-crate")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "crate": {
                        "id": "test-crate",
                        "name": "test-crate"
                    },
                    "versions": [
                        {"num": "1.0.2", "yanked": false, "created_at": "2020-03-01T00:00:00.000Z"},
                        {"num": "1.0.1", "yanked": true, "created_at": "2020-02-01T00:00:00.000Z"},
                        {"num": "1.0.0", "yanked": false, "created_at": "2020-01-01T00:00:00.000Z"}
                    ]
                }"#,
            )
            .create_async()
            .await;

        let registry = CratesIoRegistry::new(&server.url());
        let result = registry.fetch_all_versions("test-crate").await.unwrap();

        mock.assert_async().await;
        // Yanked version 1.0.1 should be excluded
        assert_eq!(
            result.versions,
            vec!["1.0.0".to_string(), "1.0.2".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_empty_for_crate_without_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/empty-crate")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "crate": {
                        "id": "empty-crate",
                        "name": "empty-crate"
                    },
                    "versions": []
                }"#,
            )
            .create_async()
            .await;

        let registry = CratesIoRegistry::new(&server.url());
        let result = registry.fetch_all_versions("empty-crate").await.unwrap();

        mock.assert_async().await;
        assert!(result.is_empty());
    }
}
