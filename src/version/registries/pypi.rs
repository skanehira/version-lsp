//! PyPI registry client for fetching Python package versions

use std::collections::HashMap;

use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;

const DEFAULT_PYPI_REGISTRY: &str = "https://pypi.org";

/// PyPI registry client
pub struct PypiRegistry {
    client: Client,
    base_url: String,
}

impl Default for PypiRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_PYPI_REGISTRY.to_string())
    }
}

impl PypiRegistry {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
        }
    }
}

/// PyPI JSON API response structure
#[derive(Debug, Deserialize)]
struct PypiResponse {
    info: PypiInfo,
    releases: HashMap<String, Vec<PypiFile>>,
}

/// Package information from PyPI
#[derive(Debug, Deserialize)]
struct PypiInfo {
    /// Latest version (according to PyPI)
    version: String,
}

/// File information (not used currently but needed for deserialization)
#[derive(Debug, Deserialize)]
struct PypiFile {
    // We don't need any fields, just need the structure to exist
    // The releases keys are what we care about
}

#[async_trait]
impl Registry for PypiRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::PyPI
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        let url = format!("{}/pypi/{}/json", self.base_url, package_name);
        debug!("Fetching PyPI package: {}", url);

        let response = self.client.get(&url).send().await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if !response.status().is_success() {
            return Err(RegistryError::InvalidResponse(format!(
                "PyPI API returned status {}",
                response.status()
            )));
        }

        let pypi_response: PypiResponse = response
            .json()
            .await
            .map_err(|e| RegistryError::InvalidResponse(e.to_string()))?;

        // Extract versions from releases keys
        let versions: Vec<String> = pypi_response.releases.keys().cloned().collect();

        // Create dist-tags with "latest" pointing to info.version
        let mut dist_tags = HashMap::new();
        dist_tags.insert("latest".to_string(), pypi_response.info.version);

        debug!(
            "Found {} versions for package {}",
            versions.len(),
            package_name
        );

        Ok(PackageVersions::with_dist_tags(versions, dist_tags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_from_releases() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/pypi/requests/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "info": {"version": "2.32.5"},
                    "releases": {
                        "2.31.0": [],
                        "2.32.0": [],
                        "2.32.5": []
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = PypiRegistry::new(server.url());
        let result = registry.fetch_all_versions("requests").await.unwrap();

        mock.assert_async().await;

        let mut versions = result.versions;
        versions.sort();
        assert_eq!(versions, vec!["2.31.0", "2.32.0", "2.32.5"]);
        assert_eq!(result.dist_tags.get("latest"), Some(&"2.32.5".to_string()));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_missing_package() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/pypi/nonexistent/json")
            .with_status(404)
            .create_async()
            .await;

        let registry = PypiRegistry::new(server.url());
        let result = registry.fetch_all_versions("nonexistent").await;

        mock.assert_async().await;

        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_handles_prerelease_versions() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("GET", "/pypi/django/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                    "info": {"version": "4.2.0"},
                    "releases": {
                        "4.1.0": [],
                        "4.2.0": [],
                        "5.0a1": [],
                        "5.0b1": [],
                        "5.0rc1": []
                    }
                }"#,
            )
            .create_async()
            .await;

        let registry = PypiRegistry::new(server.url());
        let result = registry.fetch_all_versions("django").await.unwrap();

        mock.assert_async().await;

        let mut versions = result.versions;
        versions.sort();
        assert_eq!(versions, vec!["4.1.0", "4.2.0", "5.0a1", "5.0b1", "5.0rc1"]);
    }

    #[tokio::test]
    async fn fetch_all_versions_handles_network_error() {
        // Use an invalid URL to trigger a network error
        let registry = PypiRegistry::new("http://invalid.localhost.test:99999".to_string());
        let result = registry.fetch_all_versions("requests").await;

        assert!(matches!(result, Err(RegistryError::Network(_))));
    }
}
