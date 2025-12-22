//! JSR (JavaScript Registry) API implementation

use std::collections::HashMap;

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use tracing::warn;

/// Default base URL for JSR registry
const DEFAULT_BASE_URL: &str = "https://jsr.io";

/// Response from JSR registry API
#[derive(Debug, Deserialize)]
struct JsrMetaResponse {
    #[allow(dead_code)]
    latest: Option<String>,
    versions: HashMap<String, JsrVersionMeta>,
}

/// Version metadata from JSR API
#[derive(Debug, Deserialize)]
struct JsrVersionMeta {
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(default)]
    yanked: bool,
}

/// Registry implementation for JSR registry API
#[derive(Clone)]
pub struct JsrRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl JsrRegistry {
    /// Creates a new JsrRegistry with a custom base URL
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

impl Default for JsrRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

#[async_trait::async_trait]
impl Registry for JsrRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::Jsr
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        // JSR API URL: https://jsr.io/@scope/package/meta.json
        let url = format!("{}/{}/meta.json", self.base_url, package_name);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = response.status();

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if !status.is_success() {
            warn!("JSR registry returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let meta: JsrMetaResponse = response.json().await.map_err(|e| {
            warn!("Failed to parse JSR registry response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Filter out yanked versions and sort by createdAt (oldest first)
        let mut versions: Vec<(String, Option<DateTime<Utc>>)> = meta
            .versions
            .into_iter()
            .filter(|(_, meta)| !meta.yanked)
            .map(|(v, meta)| {
                let timestamp = meta
                    .created_at
                    .and_then(|ts| DateTime::parse_from_rfc3339(&ts).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                (v, timestamp)
            })
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
    async fn fetch_all_versions_returns_versions_sorted_by_created_at() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/@luca/flag/meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "scope": "luca",
                    "name": "flag",
                    "latest": "1.0.1",
                    "versions": {
                        "1.0.1": { "createdAt": "2024-02-01T00:00:00.000Z" },
                        "1.0.0": { "createdAt": "2024-01-01T00:00:00.000Z" }
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = JsrRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@luca/flag").await.unwrap();

        mock.assert_async().await;
        assert_eq!(
            result.versions,
            vec!["1.0.0".to_string(), "1.0.1".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_filters_out_yanked_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/@std/path/meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "scope": "std",
                    "name": "path",
                    "latest": "1.0.2",
                    "versions": {
                        "1.0.0": { "createdAt": "2024-01-01T00:00:00.000Z" },
                        "1.0.1": { "createdAt": "2024-02-01T00:00:00.000Z", "yanked": true },
                        "1.0.2": { "createdAt": "2024-03-01T00:00:00.000Z" }
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = JsrRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@std/path").await.unwrap();

        mock.assert_async().await;
        assert_eq!(
            result.versions,
            vec!["1.0.0".to_string(), "1.0.2".to_string()]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_package() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/@nonexistent/package/meta.json")
            .with_status(404)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Not found"}"#)
            .create_async()
            .await;

        let registry = JsrRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@nonexistent/package").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_empty_for_package_without_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/@empty/package/meta.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "scope": "empty",
                    "name": "package",
                    "versions": {}
                }"#,
            )
            .create_async()
            .await;

        let registry = JsrRegistry::new(&server.url());
        let result = registry.fetch_all_versions("@empty/package").await.unwrap();

        mock.assert_async().await;
        assert!(result.is_empty());
    }
}
