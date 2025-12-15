//! Go proxy registry API implementation

use crate::parser::types::RegistryType;
use crate::version::error::RegistryError;
use crate::version::registry::Registry;
use crate::version::types::PackageVersions;
use tracing::warn;

/// Default base URL for Go proxy
const DEFAULT_BASE_URL: &str = "https://proxy.golang.org";

/// Registry implementation for Go proxy API
pub struct GoProxyRegistry {
    client: reqwest::Client,
    base_url: String,
}

impl GoProxyRegistry {
    /// Creates a new GoProxyRegistry with a custom base URL
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

impl Default for GoProxyRegistry {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

#[async_trait::async_trait]
impl Registry for GoProxyRegistry {
    fn registry_type(&self) -> RegistryType {
        RegistryType::GoProxy
    }

    async fn fetch_all_versions(
        &self,
        package_name: &str,
    ) -> Result<PackageVersions, RegistryError> {
        // Go proxy expects module path to be URL-encoded, with uppercase letters
        // escaped as !{lowercase}. For example: github.com/Azure -> github.com/!azure
        let encoded_module = encode_module_path(package_name);
        let url = format!("{}/{}/@v/list", self.base_url, encoded_module);

        let response = self.client.get(&url).send().await?;

        let status = response.status();

        // Go proxy returns 404 or 410 for modules that don't exist
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
            return Err(RegistryError::NotFound(package_name.to_string()));
        }

        if !status.is_success() {
            warn!("Go proxy returned status {}: {}", status, url);
            return Err(RegistryError::InvalidResponse(format!(
                "Unexpected status: {}",
                status
            )));
        }

        let body = response.text().await.map_err(|e| {
            warn!("Failed to read Go proxy response: {}", e);
            RegistryError::InvalidResponse(e.to_string())
        })?;

        // Go proxy returns versions one per line
        let versions: Vec<String> = body
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| line.to_string())
            .collect();

        Ok(PackageVersions::new(versions))
    }
}

/// Encodes a Go module path for use in proxy URLs.
/// Uppercase letters are escaped as !{lowercase}.
fn encode_module_path(path: &str) -> String {
    let mut result = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_uppercase() {
            result.push('!');
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn fetch_all_versions_returns_versions_from_proxy() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/golang.org/x/text/@v/list")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("v0.14.0\nv0.13.0\nv0.12.0\n")
            .create_async()
            .await;

        let registry = GoProxyRegistry::new(&server.url());
        let result = registry
            .fetch_all_versions("golang.org/x/text")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(
            result.versions,
            vec![
                "v0.14.0".to_string(),
                "v0.13.0".to_string(),
                "v0.12.0".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_nonexistent_module() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/nonexistent/module/@v/list")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let registry = GoProxyRegistry::new(&server.url());
        let result = registry.fetch_all_versions("nonexistent/module").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_not_found_for_gone_status() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/deprecated/module/@v/list")
            .with_status(410)
            .with_body("gone")
            .create_async()
            .await;

        let registry = GoProxyRegistry::new(&server.url());
        let result = registry.fetch_all_versions("deprecated/module").await;

        mock.assert_async().await;
        assert!(matches!(result, Err(RegistryError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_all_versions_returns_empty_for_module_without_versions() {
        let mut server = Server::new_async().await;

        let mock = server
            .mock("GET", "/empty/module/@v/list")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("")
            .create_async()
            .await;

        let registry = GoProxyRegistry::new(&server.url());
        let result = registry.fetch_all_versions("empty/module").await.unwrap();

        mock.assert_async().await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_all_versions_handles_uppercase_module_path() {
        let mut server = Server::new_async().await;

        // Go proxy encodes uppercase as !{lowercase}
        let mock = server
            .mock("GET", "/github.com/!azure/azure-sdk-for-go/@v/list")
            .with_status(200)
            .with_header("content-type", "text/plain")
            .with_body("v1.0.0\n")
            .create_async()
            .await;

        let registry = GoProxyRegistry::new(&server.url());
        let result = registry
            .fetch_all_versions("github.com/Azure/azure-sdk-for-go")
            .await
            .unwrap();

        mock.assert_async().await;
        assert_eq!(result.versions, vec!["v1.0.0".to_string()]);
    }

    #[test]
    fn encode_module_path_escapes_uppercase_letters() {
        assert_eq!(encode_module_path("github.com/Azure"), "github.com/!azure");
        assert_eq!(
            encode_module_path("github.com/Azure/AzureSDK"),
            "github.com/!azure/!azure!s!d!k"
        );
        assert_eq!(encode_module_path("golang.org/x/text"), "golang.org/x/text");
    }
}
