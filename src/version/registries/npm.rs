//! npm registry API implementation

use std::collections::HashMap;

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use semver::Version;
use serde::Deserialize;
use tracing::warn;

/// Default base URL for npm registry
const DEFAULT_BASE_URL: &str = "https://registry.npmjs.org";

/// Response from npm registry API
#[derive(Debug, Deserialize)]
struct NpmPackageResponse {
    versions: HashMap<String, serde_json::Value>,
}

/// Registry implementation for npm registry API
pub struct NpmRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl NpmRegistry {
    /// Creates a new NpmRegistry with a custom base URL
    pub fn new(base_url: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent("version-lsp")
                .build()
                .expect("Failed to create HTTP client"),
            base_url: base_url.to_string(),
        }
    }

    /// Encode package name for URL (handles scoped packages)
    fn encode_package_name(package_name: &str) -> String {
        if package_name.starts_with('@') {
            // Scoped package: @scope/name -> @scope%2Fname
            package_name.replace('/', "%2F")
        } else {
            package_name.to_string()
        }
    }
}

impl Default for NpmRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

#[async_trait::async_trait]
impl Registry for NpmRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Npm
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        let encoded_name = Self::encode_package_name(package_name);
        let url = format!("{}/{}", self.base_url, encoded_name);

        let response = self.client.get(&url).send().await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if !status.is_success() {
            warn!("npm registry returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let package_info: NpmPackageResponse = response.json().await.map_err(|e| {
            warn!("Failed to parse npm registry response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Sort versions by semver (lowest first, highest last)
        let mut versions: Vec<(String, Version)> = package_info
            .versions
            .into_keys()
            .filter_map(|v| Version::parse(&v).ok().map(|parsed| (v, parsed)))
            .collect();

        versions.sort_by(|(_, a), (_, b)| a.cmp(b));

        let versions: Vec<String> = versions.into_iter().map(|(v, _)| v).collect();

        Ok(PackageVersions::new(versions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_sorted_by_semver() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/lodash")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "name": "lodash",
                    "versions": {
                        "4.17.21": {},
                        "4.17.19": {},
                        "4.17.20": {}
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("lodash").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by semver (lowest first, highest last)
        assert_eq!(
            result.versions,
            vec![
                "4.17.19".to_string(),
                "4.17.20".to_string(),
                "4.17.21".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_package() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/nonexistent-package")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Not found"}"#)
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("nonexistent-package").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_handles_scoped_package() {
        let mut server = Server::new_async().await;

        // Scoped packages use URL encoding: @types/node -> @types%2Fnode
        let mock = server
            .mock("GET", "/@types%2Fnode")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "name": "@types/node",
                    "versions": {
                        "20.0.0": {},
                        "18.0.0": {}
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@types/node").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by semver
        assert_eq!(
            result.versions,
            vec!["18.0.0".to_string(), "20.0.0".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_empty_for_package_without_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/empty-package")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "name": "empty-package",
                    "versions": {}
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("empty-package").await.unwrap();

        mock.assert_async().await;
        assert!(result.is_empty());
    }
}
