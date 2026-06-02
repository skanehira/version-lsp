use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Database lock poisoned")]
    LockPoisoned,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Rate limited: retry after {retry_after_secs:?} seconds")]
    RateLimited { retry_after_secs: Option<u64> },

    #[error("Package not found: {0}")]
    NotFound(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),
}

#[derive(Debug, Error)]
pub enum LockError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid lock file format: {0}")]
    InvalidFormat(String),

    #[error("Manifest URI is not a local file: {0}")]
    InvalidManifestUri(String),
}
