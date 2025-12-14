use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use tracing::{debug, info};

use crate::version::checker::VersionStorer;
use crate::version::error::CacheError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageId {
    pub registry_type: String,
    pub package_name: String,
}

pub struct Cache {
    conn: Mutex<Connection>,
    refresh_interval: i64,
}

impl Cache {
    pub fn new(db_path: &Path, refresh_interval: i64) -> Result<Self, CacheError> {
        info!("Initializing cache database at {:?}", db_path);

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        debug!("Database connection established");

        let cache = Self {
            conn: Mutex::new(conn),
            refresh_interval,
        };

        cache.create_schema()?;
        info!("Cache initialized successfully");

        Ok(cache)
    }

    /// Timeout for fetch operations in milliseconds (30 seconds)
    const FETCH_TIMEOUT_MS: i64 = 30_000;

    fn create_schema(&self) -> Result<(), CacheError> {
        debug!("Creating database schema");

        let conn = self.conn.lock().unwrap();

        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                registry_type TEXT NOT NULL,
                package_name TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                fetching_since INTEGER,
                UNIQUE(registry_type, package_name)
            )
            "#,
            [],
        )?;

        // Migration: Add fetching_since column if it doesn't exist
        let has_fetching_since: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('packages') WHERE name = 'fetching_since'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_fetching_since {
            conn.execute("ALTER TABLE packages ADD COLUMN fetching_since INTEGER", [])?;
            debug!("Added fetching_since column to packages table");
        }

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_updated_at ON packages(updated_at)",
            [],
        )?;

        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS versions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_id INTEGER NOT NULL,
                version TEXT NOT NULL,
                FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
                UNIQUE(package_id, version)
            )
            "#,
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_package_id ON versions(package_id)",
            [],
        )?;

        debug!("Database schema created successfully");
        Ok(())
    }

    pub fn get_versions(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
            SELECT v.version FROM versions v
            JOIN packages p ON v.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2
            "#,
        )?;

        let versions = stmt
            .query_map((registry_type, package_name), |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(versions)
    }
}

impl VersionStorer for Cache {
    fn get_latest_version(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Option<String>, CacheError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            r#"
            SELECT v.version FROM versions v
            JOIN packages p ON v.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2
            ORDER BY v.id DESC
            LIMIT 1
            "#,
            (registry_type, package_name),
            |row| row.get(0),
        );

        match result {
            Ok(version) => Ok(Some(version)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn get_versions(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError> {
        Cache::get_versions(self, registry_type, package_name)
    }

    fn version_exists(
        &self,
        registry_type: &str,
        package_name: &str,
        version: &str,
    ) -> Result<bool, CacheError> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn.query_row(
            r#"
            SELECT EXISTS(
                SELECT 1 FROM versions v
                JOIN packages p ON v.package_id = p.id
                WHERE p.registry_type = ?1 AND p.package_name = ?2 AND v.version = ?3
            )
            "#,
            (registry_type, package_name, version),
            |row| row.get(0),
        )?;

        Ok(exists)
    }

    fn replace_versions(
        &self,
        registry_type: &str,
        package_name: &str,
        versions: Vec<String>,
    ) -> Result<(), CacheError> {
        debug!(
            "Saving {} versions for {}/{}",
            versions.len(),
            registry_type,
            package_name
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        // Insert or update package
        tx.execute(
            r#"
            INSERT INTO packages (registry_type, package_name, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(registry_type, package_name) DO UPDATE SET updated_at = excluded.updated_at
            "#,
            (registry_type, package_name, now),
        )?;

        // Get package_id
        let package_id: i64 = tx.query_row(
            "SELECT id FROM packages WHERE registry_type = ?1 AND package_name = ?2",
            (registry_type, package_name),
            |row| row.get(0),
        )?;

        // Delete existing versions
        tx.execute("DELETE FROM versions WHERE package_id = ?1", [package_id])?;

        // Insert new versions
        {
            let mut stmt =
                tx.prepare("INSERT INTO versions (package_id, version) VALUES (?1, ?2)")?;
            for version in &versions {
                stmt.execute((package_id, version))?;
            }
        }

        tx.commit()?;

        debug!(
            "Successfully saved versions for {}/{}",
            registry_type, package_name
        );
        Ok(())
    }

    fn get_packages_needing_refresh(&self) -> Result<Vec<PackageId>, CacheError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let threshold = now - self.refresh_interval;

        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT registry_type, package_name FROM packages WHERE updated_at < ?1")?;

        let packages = stmt
            .query_map([threshold], |row| {
                Ok(PackageId {
                    registry_type: row.get(0)?,
                    package_name: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<PackageId>, _>>()?;

        Ok(packages)
    }

    fn try_start_fetch(&self, registry_type: &str, package_name: &str) -> Result<bool, CacheError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let timeout_threshold = now - Cache::FETCH_TIMEOUT_MS;

        let conn = self.conn.lock().unwrap();

        // Try to set fetching_since if:
        // 1. Package doesn't exist (will be created by replace_versions later)
        // 2. fetching_since is NULL (not being fetched)
        // 3. fetching_since is older than timeout (previous fetch timed out)
        let rows_affected = conn.execute(
            r#"
            UPDATE packages
            SET fetching_since = ?1
            WHERE registry_type = ?2 AND package_name = ?3
              AND (fetching_since IS NULL OR fetching_since < ?4)
            "#,
            (now, registry_type, package_name, timeout_threshold),
        )?;

        if rows_affected > 0 {
            return Ok(true);
        }

        // Package might not exist yet, check if we can proceed
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM packages WHERE registry_type = ?1 AND package_name = ?2)",
            (registry_type, package_name),
            |row| row.get(0),
        )?;

        // If package doesn't exist, we can proceed (it will be created by replace_versions)
        Ok(!exists)
    }

    fn finish_fetch(&self, registry_type: &str, package_name: &str) -> Result<(), CacheError> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE packages SET fetching_since = NULL WHERE registry_type = ?1 AND package_name = ?2",
            (registry_type, package_name),
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    #[test]
    fn replace_versions_creates_new_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let versions = vec![
            "1.0.0".to_string(),
            "1.1.0".to_string(),
            "2.0.0".to_string(),
        ];
        cache
            .replace_versions("npm", "axios", versions.clone())
            .unwrap();

        let saved = cache.get_versions("npm", "axios").unwrap();
        assert_eq!(saved, versions);
    }

    #[test]
    fn replace_versions_updates_existing_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let initial_versions = vec!["1.0.0".to_string()];
        cache
            .replace_versions("npm", "axios", initial_versions)
            .unwrap();

        let new_versions = vec!["1.0.0".to_string(), "1.1.0".to_string()];
        cache
            .replace_versions("npm", "axios", new_versions.clone())
            .unwrap();

        let saved = cache.get_versions("npm", "axios").unwrap();
        assert_eq!(saved, new_versions);
    }

    #[test]
    fn get_versions_returns_empty_for_nonexistent_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let versions = cache.get_versions("npm", "nonexistent").unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn get_versions_performance_with_1000_versions() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let versions: Vec<String> = (0..1000).map(|i| format!("{}.0.0", i)).collect();
        cache
            .replace_versions("npm", "large-package", versions.clone())
            .unwrap();

        let start = std::time::Instant::now();
        let retrieved = cache.get_versions("npm", "large-package").unwrap();
        let elapsed = start.elapsed();

        assert_eq!(retrieved.len(), 1000);
        assert!(
            elapsed.as_millis() < 10,
            "get_versions took {}ms, expected < 10ms",
            elapsed.as_millis()
        );
    }

    #[rstest]
    #[case("npm", "axios", "1.0.0", true)]
    #[case("npm", "axios", "2.0.0", true)]
    #[case("npm", "axios", "9.9.9", false)]
    #[case("npm", "nonexistent", "1.0.0", false)]
    #[case("crates", "axios", "1.0.0", false)]
    fn version_exists_returns_expected(
        #[case] registry_type: &str,
        #[case] package_name: &str,
        #[case] version: &str,
        #[case] expected: bool,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let versions = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        cache.replace_versions("npm", "axios", versions).unwrap();

        assert_eq!(
            cache
                .version_exists(registry_type, package_name, version)
                .unwrap(),
            expected
        );
    }

    #[rstest]
    #[case("npm", "axios", Some("3.0.0".to_string()))]
    #[case("npm", "nonexistent", None)]
    #[case("crates", "axios", None)]
    fn get_latest_version_returns_last_inserted(
        #[case] registry_type: &str,
        #[case] package_name: &str,
        #[case] expected: Option<String>,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
        ];
        cache.replace_versions("npm", "axios", versions).unwrap();

        assert_eq!(
            cache
                .get_latest_version(registry_type, package_name)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn get_packages_needing_refresh_returns_packages_older_than_refresh_interval() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 100ms
        let cache = Cache::new(&db_path, 100).unwrap();

        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();
        cache
            .replace_versions("npm", "lodash", vec!["4.0.0".to_string()])
            .unwrap();

        // Wait for packages to become stale
        std::thread::sleep(std::time::Duration::from_millis(150));

        let stale = cache.get_packages_needing_refresh().unwrap();
        assert_eq!(stale.len(), 2);
        assert!(stale.contains(&PackageId {
            registry_type: "npm".to_string(),
            package_name: "axios".to_string()
        }));
        assert!(stale.contains(&PackageId {
            registry_type: "npm".to_string(),
            package_name: "lodash".to_string()
        }));
    }

    #[test]
    fn get_packages_needing_refresh_excludes_fresh_packages() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 1 hour (in ms)
        let cache = Cache::new(&db_path, 3600000).unwrap();

        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let stale = cache.get_packages_needing_refresh().unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn try_start_fetch_returns_true_for_new_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        // New package not in DB should allow fetch
        let can_fetch = cache.try_start_fetch("npm", "new-package").unwrap();
        assert!(can_fetch);
    }

    #[test]
    fn try_start_fetch_returns_true_for_package_not_being_fetched() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        // Pre-populate cache (fetching_since is NULL after replace_versions)
        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Package exists but not being fetched should allow fetch
        let can_fetch = cache.try_start_fetch("npm", "axios").unwrap();
        assert!(can_fetch);
    }

    #[test]
    fn try_start_fetch_returns_false_for_package_being_fetched() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        // Pre-populate cache
        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // First fetch should succeed
        let can_fetch1 = cache.try_start_fetch("npm", "axios").unwrap();
        assert!(can_fetch1);

        // Second fetch should fail (already being fetched)
        let can_fetch2 = cache.try_start_fetch("npm", "axios").unwrap();
        assert!(!can_fetch2);
    }

    #[test]
    fn finish_fetch_clears_fetching_state() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        // Pre-populate cache
        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Start fetch
        let can_fetch1 = cache.try_start_fetch("npm", "axios").unwrap();
        assert!(can_fetch1);

        // Finish fetch
        cache.finish_fetch("npm", "axios").unwrap();

        // Should be able to fetch again
        let can_fetch2 = cache.try_start_fetch("npm", "axios").unwrap();
        assert!(can_fetch2);
    }
}
