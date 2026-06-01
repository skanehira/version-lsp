//! npm registry API implementation

use std::collections::HashMap;

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::warn;

/// Default base URL for npm registry
const DEFAULT_BASE_URL: &str = "https://registry.npmjs.org";

/// Response from npm registry API
#[derive(Debug, Deserialize)]
struct NpmPackageResponse {
    versions: HashMap<String, serde_json::Value>,
    #[serde(rename = "dist-tags", default)]
    dist_tags: HashMap<String, String>,
    /// Version publish timestamps (version -> ISO 8601 timestamp)
    #[serde(default)]
    time: HashMap<String, String>,
}

/// Registry implementation for npm registry API
#[derive(Clone)]
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

        // Sort versions by publish date (oldest first, newest last)
        // Versions without timestamps are placed at the beginning
        let mut versions: Vec<(String, Option<DateTime<Utc>>)> = package_info
            .versions
            .into_keys()
            .map(|v| {
                let timestamp = package_info
                    .time
                    .get(&v)
                    .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                (v, timestamp)
            })
            .collect();

        versions.sort_by_key(|(_, a)| *a);

        let versions: Vec<String> = versions.into_iter().map(|(v, _)| v).collect();

        Ok(PackageVersions::with_dist_tags(
            versions,
            package_info.dist_tags,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_sorted_by_time() {
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
                    },
                    "time": {
                        "4.17.19": "2020-07-08T17:14:40.866Z",
                        "4.17.20": "2020-08-13T16:53:54.152Z",
                        "4.17.21": "2021-02-20T15:42:16.891Z"
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("lodash").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by publish date (oldest first, newest last)
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
                    },
                    "time": {
                        "18.0.0": "2022-06-01T00:00:00.000Z",
                        "20.0.0": "2023-04-01T00:00:00.000Z"
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@types/node").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by publish date
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

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_sorted_by_publish_date() {
        let mut server = Server::new_async().await;

        // Intentionally set time field so that publish date order differs from semver order
        // semver order: 1.0.0 < 1.5.0 < 2.0.0
        // publish date order: 1.0.0 (Jan) < 2.0.0 (Jun) < 1.5.0 (Dec)
        let mock = server
            .mock("GET", "/test-pkg")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "name": "test-pkg",
                    "versions": {
                        "1.0.0": {},
                        "2.0.0": {},
                        "1.5.0": {}
                    },
                    "time": {
                        "1.0.0": "2020-01-01T00:00:00.000Z",
                        "2.0.0": "2020-06-01T00:00:00.000Z",
                        "1.5.0": "2020-12-01T00:00:00.000Z"
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("test-pkg").await.unwrap();

        mock.assert_async().await;
        // Versions should be sorted by publish date (oldest first, newest last)
        assert_eq!(
            result.versions,
            vec![
                "1.0.0".to_string(),
                "2.0.0".to_string(),
                "1.5.0".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_dist_tags() {
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
                        "4.17.20": {},
                        "5.0.0-beta.1": {}
                    },
                    "dist-tags": {
                        "latest": "4.17.21",
                        "beta": "5.0.0-beta.1"
                    },
                    "time": {
                        "4.17.20": "2020-08-13T16:53:54.152Z",
                        "4.17.21": "2021-02-20T15:42:16.891Z",
                        "5.0.0-beta.1": "2021-06-01T00:00:00.000Z"
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = NpmRegistry::new(&server.url());
        let result = registry.fetch_all_versions("lodash").await.unwrap();

        mock.assert_async().await;

        // Verify dist-tags were extracted
        assert_eq!(result.dist_tags.get("latest"), Some(&"4.17.21".to_string()));
        assert_eq!(
            result.dist_tags.get("beta"),
            Some(&"5.0.0-beta.1".to_string())
        );
    }
}
