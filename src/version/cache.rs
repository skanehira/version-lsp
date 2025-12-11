#![allow(dead_code)]
use std::path::Path;

use rusqlite::Connection;
use tracing::{debug, info};

use crate::version::error::CacheError;

pub struct Cache {
    conn: Connection,
    #[allow(dead_code)]
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
            conn,
            refresh_interval,
        };

        cache.create_schema()?;
        info!("Cache initialized successfully");

        Ok(cache)
    }

    fn create_schema(&self) -> Result<(), CacheError> {
        debug!("Creating database schema");

        self.conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS packages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                registry_type TEXT NOT NULL,
                package_name TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(registry_type, package_name)
            )
            "#,
            [],
        )?;

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_updated_at ON packages(updated_at)",
            [],
        )?;

        self.conn.execute(
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

        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_package_id ON versions(package_id)",
            [],
        )?;

        debug!("Database schema created successfully");
        Ok(())
    }

    pub fn replace_versions(
        &mut self,
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

        let tx = self.conn.transaction()?;

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

    pub fn get_versions(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError> {
        let mut stmt = self.conn.prepare(
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

    pub fn version_exists(
        &self,
        registry_type: &str,
        package_name: &str,
        version: &str,
    ) -> Result<bool, CacheError> {
        let exists: bool = self.conn.query_row(
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

    pub fn get_latest_version(
        &self,
        registry_type: &str,
        package_name: &str,
    ) -> Result<Option<String>, CacheError> {
        let result = self.conn.query_row(
            r#"
            SELECT v.version FROM versions v
            JOIN packages p ON v.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2
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

    pub fn get_stale_packages(&self) -> Result<Vec<(String, String)>, CacheError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let threshold = now - self.refresh_interval;

        let mut stmt = self
            .conn
            .prepare("SELECT registry_type, package_name FROM packages WHERE updated_at < ?1")?;

        let packages = stmt
            .query_map([threshold], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, String)>, _>>()?;

        Ok(packages)
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
        let mut cache = Cache::new(&db_path, 86400).unwrap();

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
        let mut cache = Cache::new(&db_path, 86400).unwrap();

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
        let mut cache = Cache::new(&db_path, 86400).unwrap();

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
        let mut cache = Cache::new(&db_path, 86400).unwrap();

        let versions = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        cache.replace_versions("npm", "axios", versions).unwrap();

        assert_eq!(
            cache
                .version_exists(registry_type, package_name, version)
                .unwrap(),
            expected
        );
    }

    #[test]
    fn get_latest_version_returns_first_version() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let mut cache = Cache::new(&db_path, 86400).unwrap();

        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
        ];
        cache.replace_versions("npm", "axios", versions).unwrap();

        let latest = cache.get_latest_version("npm", "axios").unwrap();
        assert_eq!(latest, Some("1.0.0".to_string()));
    }

    #[test]
    fn get_latest_version_returns_none_for_nonexistent_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400).unwrap();

        let latest = cache.get_latest_version("npm", "nonexistent").unwrap();
        assert_eq!(latest, None);
    }

    #[test]
    fn get_stale_packages_returns_packages_older_than_refresh_interval() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 100ms
        let mut cache = Cache::new(&db_path, 100).unwrap();

        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();
        cache
            .replace_versions("npm", "lodash", vec!["4.0.0".to_string()])
            .unwrap();

        // Wait for packages to become stale
        std::thread::sleep(std::time::Duration::from_millis(150));

        let stale = cache.get_stale_packages().unwrap();
        assert_eq!(stale.len(), 2);
        assert!(stale.contains(&("npm".to_string(), "axios".to_string())));
        assert!(stale.contains(&("npm".to_string(), "lodash".to_string())));
    }

    #[test]
    fn get_stale_packages_excludes_fresh_packages() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 1 hour (in ms)
        let mut cache = Cache::new(&db_path, 3600000).unwrap();

        cache
            .replace_versions("npm", "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let stale = cache.get_stale_packages().unwrap();
        assert!(stale.is_empty());
    }
}
