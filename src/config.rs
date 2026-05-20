use serde::Deserialize;
use std::fmt;
use std::path::PathBuf;

// =============================================================================
// Time-related constants
// =============================================================================

/// Default refresh interval in milliseconds (24 hours)
pub const DEFAULT_REFRESH_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;

/// Timeout for fetch operations in milliseconds (30 seconds)
pub const FETCH_TIMEOUT_MS: i64 = 30_000;

/// Delay between starting each fetch request to avoid rate limiting (10ms)
pub const FETCH_STAGGER_DELAY_MS: u64 = 10;

/// LSP configuration structure
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LspConfig {
    pub cache: CacheConfig,
    pub registries: RegistriesConfig,
    /// Whether to ignore prerelease versions when determining the latest version
    pub ignore_prerelease: bool,
}

impl Default for LspConfig {
    fn default() -> Self {
        Self {
            cache: CacheConfig::default(),
            registries: RegistriesConfig::default(),
            ignore_prerelease: true,
        }
    }
}

/// Cache-related configuration
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CacheConfig {
    /// Cache refresh interval in milliseconds
    pub refresh_interval: i64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            refresh_interval: DEFAULT_REFRESH_INTERVAL_MS,
        }
    }
}

/// Registry-specific configuration
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct RegistriesConfig {
    pub npm: RegistryConfig,
    pub crates: RegistryConfig,
    #[serde(rename = "goProxy")]
    pub go_proxy: RegistryConfig,
    pub github: RegistryConfig,
    #[serde(rename = "pnpmCatalog")]
    pub pnpm_catalog: RegistryConfig,
    pub jsr: RegistryConfig,
    pub pypi: RegistryConfig,
    pub docker: DockerRegistryConfig,
}

/// Individual registry configuration with optional URL override
#[derive(Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct RegistryConfig {
    pub enabled: bool,
    /// Override the registry base URL. When `None`, the registry's hardcoded default is used.
    pub url: Option<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            url: None,
        }
    }
}

impl fmt::Debug for RegistryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryConfig")
            .field("enabled", &self.enabled)
            .field("url", &self.url.as_deref().map(redact_userinfo))
            .finish()
    }
}

/// Docker registry configuration. Docker dispatches to either Docker Hub or
/// ghcr.io based on the image name prefix, so each backend has its own
/// optional registry and auth URL override.
#[derive(Clone, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct DockerRegistryConfig {
    pub enabled: bool,
    pub docker_hub_registry_url: Option<String>,
    pub docker_hub_auth_url: Option<String>,
    pub ghcr_registry_url: Option<String>,
    pub ghcr_auth_url: Option<String>,
}

impl Default for DockerRegistryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            docker_hub_registry_url: None,
            docker_hub_auth_url: None,
            ghcr_registry_url: None,
            ghcr_auth_url: None,
        }
    }
}

impl fmt::Debug for DockerRegistryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockerRegistryConfig")
            .field("enabled", &self.enabled)
            .field(
                "docker_hub_registry_url",
                &self.docker_hub_registry_url.as_deref().map(redact_userinfo),
            )
            .field(
                "docker_hub_auth_url",
                &self.docker_hub_auth_url.as_deref().map(redact_userinfo),
            )
            .field(
                "ghcr_registry_url",
                &self.ghcr_registry_url.as_deref().map(redact_userinfo),
            )
            .field(
                "ghcr_auth_url",
                &self.ghcr_auth_url.as_deref().map(redact_userinfo),
            )
            .finish()
    }
}

/// Replace `user:password@` userinfo in a URL with `***@` so credentials are
/// not leaked through `Debug` formatting (e.g. `tracing::info!("{:?}", config)`).
///
/// Non-URL strings or URLs without userinfo are returned unchanged. This is a
/// best-effort textual transform; it does not parse the URL.
fn redact_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after_scheme = scheme_end + 3;
    let rest = &url[after_scheme..];

    // Userinfo, if present, ends at the first '@' before the next '/', '?' or '#'.
    let host_terminator = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..host_terminator];

    let Some(at_idx) = authority.find('@') else {
        return url.to_string();
    };

    let mut redacted = String::with_capacity(url.len());
    redacted.push_str(&url[..after_scheme]);
    redacted.push_str("***");
    redacted.push_str(&authority[at_idx..]);
    redacted.push_str(&rest[host_terminator..]);
    redacted
}

/// Returns the path to the data directory for version-lsp.
/// Uses $XDG_DATA_HOME/version-lsp if XDG_DATA_HOME is set,
/// otherwise falls back to ~/.local/share/version-lsp,
/// or ./version-lsp if neither is available.
pub fn data_dir() -> PathBuf {
    data_dir_with_env(std::env::var("XDG_DATA_HOME").ok(), dirs::home_dir())
}

/// Returns the path to the database file.
pub fn db_path() -> PathBuf {
    data_dir().join("versions.db")
}

/// Returns the path to the log file.
pub fn log_path() -> PathBuf {
    data_dir().join("version-lsp.log")
}

fn data_dir_with_env(xdg_data_home: Option<String>, home_dir: Option<PathBuf>) -> PathBuf {
    let data_dir = xdg_data_home
        .map(PathBuf::from)
        .or_else(|| home_dir.map(|home| home.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));

    data_dir.join("version-lsp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lsp_config_from_partial_object_uses_defaults_for_missing_fields() {
        let result = serde_json::from_value::<LspConfig>(json!({
            "cache": {
                "refreshInterval": 1000
            }
        }))
        .unwrap();

        assert_eq!(result.cache.refresh_interval, 1000);
        assert_eq!(result.registries, RegistriesConfig::default());
    }

    #[test]
    fn lsp_config_from_full_object_parses_all_fields() {
        let result = serde_json::from_value::<LspConfig>(json!({
            "cache": {
                "refreshInterval": 5000
            },
            "registries": {
                "npm": { "enabled": false },
                "crates": { "enabled": true },
                "goProxy": { "enabled": false },
                "github": { "enabled": true },
                "pnpmCatalog": { "enabled": false },
                "jsr": { "enabled": false },
                "pypi": { "enabled": true }
            }
        }))
        .unwrap();

        assert_eq!(
            result,
            LspConfig {
                cache: CacheConfig {
                    refresh_interval: 5000
                },
                registries: RegistriesConfig {
                    npm: RegistryConfig {
                        enabled: false,
                        url: None
                    },
                    crates: RegistryConfig {
                        enabled: true,
                        url: None
                    },
                    go_proxy: RegistryConfig {
                        enabled: false,
                        url: None
                    },
                    github: RegistryConfig {
                        enabled: true,
                        url: None
                    },
                    pnpm_catalog: RegistryConfig {
                        enabled: false,
                        url: None
                    },
                    jsr: RegistryConfig {
                        enabled: false,
                        url: None
                    },
                    pypi: RegistryConfig {
                        enabled: true,
                        url: None
                    },
                    docker: DockerRegistryConfig::default(),
                },
                ignore_prerelease: true,
            }
        );
    }

    #[test]
    fn registry_config_parses_url_override() {
        let result = serde_json::from_value::<LspConfig>(json!({
            "registries": {
                "pypi": { "url": "https://private.example.com/simple" },
                "npm": { "enabled": false, "url": "https://npm.internal/" }
            }
        }))
        .unwrap();

        assert_eq!(
            result.registries.pypi,
            RegistryConfig {
                enabled: true,
                url: Some("https://private.example.com/simple".to_string())
            }
        );
        assert_eq!(
            result.registries.npm,
            RegistryConfig {
                enabled: false,
                url: Some("https://npm.internal/".to_string())
            }
        );
    }

    #[test]
    fn docker_registry_config_parses_all_url_overrides() {
        let result = serde_json::from_value::<LspConfig>(json!({
            "registries": {
                "docker": {
                    "dockerHubRegistryUrl": "https://hub.example.com",
                    "dockerHubAuthUrl": "https://auth.example.com/token",
                    "ghcrRegistryUrl": "https://ghcr.internal",
                    "ghcrAuthUrl": "https://ghcr.internal/token"
                }
            }
        }))
        .unwrap();

        assert_eq!(
            result.registries.docker,
            DockerRegistryConfig {
                enabled: true,
                docker_hub_registry_url: Some("https://hub.example.com".to_string()),
                docker_hub_auth_url: Some("https://auth.example.com/token".to_string()),
                ghcr_registry_url: Some("https://ghcr.internal".to_string()),
                ghcr_auth_url: Some("https://ghcr.internal/token".to_string()),
            }
        );
    }

    #[test]
    fn registry_config_debug_redacts_userinfo() {
        let config = RegistryConfig {
            enabled: true,
            url: Some("https://user:secret@private.example.com/simple".to_string()),
        };

        let debug = format!("{:?}", config);

        assert!(!debug.contains("secret"), "password leaked in: {}", debug);
        assert!(!debug.contains("user:"), "username leaked in: {}", debug);
        assert!(debug.contains("***@private.example.com"));
    }

    #[test]
    fn docker_registry_config_debug_redacts_userinfo() {
        let config = DockerRegistryConfig {
            enabled: true,
            docker_hub_registry_url: Some("https://u:p@hub.example.com".to_string()),
            docker_hub_auth_url: None,
            ghcr_registry_url: Some("https://x:y@ghcr.internal/".to_string()),
            ghcr_auth_url: None,
        };

        let debug = format!("{:?}", config);

        assert!(!debug.contains(":p@"), "password leaked in: {}", debug);
        assert!(!debug.contains(":y@"), "password leaked in: {}", debug);
        assert!(debug.contains("***@hub.example.com"));
        assert!(debug.contains("***@ghcr.internal"));
    }

    #[test]
    fn redact_userinfo_passes_through_urls_without_credentials() {
        assert_eq!(
            redact_userinfo("https://pypi.org/simple"),
            "https://pypi.org/simple"
        );
        assert_eq!(
            redact_userinfo("https://example.com/path?q=1#frag"),
            "https://example.com/path?q=1#frag"
        );
    }

    #[test]
    fn redact_userinfo_redacts_user_and_password() {
        assert_eq!(
            redact_userinfo("https://alice:hunter2@example.com/path"),
            "https://***@example.com/path"
        );
    }

    #[test]
    fn redact_userinfo_redacts_user_only() {
        assert_eq!(
            redact_userinfo("https://token@example.com/"),
            "https://***@example.com/"
        );
    }

    #[test]
    fn redact_userinfo_does_not_treat_at_in_path_as_userinfo() {
        // The '@' is in the path, not the authority.
        assert_eq!(
            redact_userinfo("https://example.com/foo@bar"),
            "https://example.com/foo@bar"
        );
    }

    #[test]
    fn redact_userinfo_returns_input_when_not_a_url() {
        assert_eq!(redact_userinfo("not-a-url"), "not-a-url");
    }

    #[test]
    fn data_dir_with_env_uses_xdg_data_home_when_set() {
        let path = data_dir_with_env(
            Some("/tmp/test-data".to_string()),
            Some(PathBuf::from("/home/user")),
        );

        assert_eq!(path, PathBuf::from("/tmp/test-data/version-lsp"));
    }

    #[test]
    fn data_dir_with_env_falls_back_to_home_local_share() {
        let path = data_dir_with_env(None, Some(PathBuf::from("/home/user")));

        assert_eq!(path, PathBuf::from("/home/user/.local/share/version-lsp"));
    }

    #[test]
    fn data_dir_with_env_falls_back_to_current_dir_when_no_dirs_available() {
        let path = data_dir_with_env(None, None);
        assert_eq!(path, PathBuf::from("./version-lsp"));
    }
}
