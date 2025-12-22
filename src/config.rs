use serde::Deserialize;
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
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LspConfig {
    pub cache: CacheConfig,
    pub registries: RegistriesConfig,
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
}

/// Individual registry configuration
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct RegistryConfig {
    pub enabled: bool,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
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
                "jsr": { "enabled": false }
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
                    npm: RegistryConfig { enabled: false },
                    crates: RegistryConfig { enabled: true },
                    go_proxy: RegistryConfig { enabled: false },
                    github: RegistryConfig { enabled: true },
                    pnpm_catalog: RegistryConfig { enabled: false },
                    jsr: RegistryConfig { enabled: false },
                }
            }
        );
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
