use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;
use tracing::{debug, info};

use crate::config::FETCH_TIMEOUT_MS;
use crate::parser::types::RegistryType;
use crate::version::checker::VersionStorer;
use crate::version::error::CacheError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageId {
    pub registry_type: RegistryType,
    pub package_name: String,
}

/// Schema migrations
/// Each version contains a list of SQL statements to execute
const MIGRATIONS: &[&[&str]] = &[
    // v1: fetching_since column
    &["ALTER TABLE packages ADD COLUMN fetching_since INTEGER"],
    // v2: not_found column
    &["ALTER TABLE packages ADD COLUMN not_found INTEGER NOT NULL DEFAULT 0"],
];

pub struct Cache {
    conn: Mutex<Connection>,
    refresh_interval: i64,
    ignore_prerelease: bool,
}

impl Cache {
    pub fn new(
        db_path: &Path,
        refresh_interval: i64,
        ignore_prerelease: bool,
    ) -> Result<Self, CacheError> {
        info!("Initializing cache database at {:?}", db_path);

        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        debug!("Database connection established");

        let cache = Self {
            conn: Mutex::new(conn),
            refresh_interval,
            ignore_prerelease,
        };

        cache.create_schema()?;
        info!("Cache initialized successfully");

        Ok(cache)
    }

    /// Acquire database connection lock with proper error handling
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, CacheError> {
        self.conn.lock().map_err(|_| CacheError::LockPoisoned)
    }

    /// Get current timestamp in milliseconds since UNIX epoch
    fn current_timestamp_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before UNIX epoch")
            .as_millis() as i64
    }

    fn create_schema(&self) -> Result<(), CacheError> {
        debug!("Creating database schema");

        let conn = self.lock_conn()?;

        // Create base tables (without migration columns)
        conn.execute(
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

        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS dist_tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                package_id INTEGER NOT NULL,
                tag_name TEXT NOT NULL,
                version TEXT NOT NULL,
                FOREIGN KEY (package_id) REFERENCES packages(id) ON DELETE CASCADE,
                UNIQUE(package_id, tag_name)
            )
            "#,
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_dist_tags_package_id ON dist_tags(package_id)",
            [],
        )?;

        // Apply migrations
        Self::apply_migrations(&conn)?;

        debug!("Database schema created successfully");
        Ok(())
    }

    /// Apply pending migrations based on user_version pragma
    fn apply_migrations(conn: &Connection) -> Result<(), CacheError> {
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        for (i, statements) in MIGRATIONS.iter().enumerate() {
            let version = (i + 1) as i32;
            if version > current_version {
                for sql in *statements {
                    // Handle "duplicate column name" error for existing DBs
                    // that were created before the migration system
                    match conn.execute(sql, []) {
                        Ok(_) => {}
                        Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
                            if msg.contains("duplicate column name") =>
                        {
                            debug!("Column already exists, skipping: {}", sql);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                debug!("Applied migration v{}", version);
            }
        }

        let target_version = MIGRATIONS.len() as i32;
        if target_version > current_version {
            conn.pragma_update(None, "user_version", target_version)?;
            debug!("Updated schema version to v{}", target_version);
        }

        Ok(())
    }

    pub fn get_versions(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError> {
        let registry_type_str = registry_type.as_str();
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            r#"
            SELECT v.version FROM versions v
            JOIN packages p ON v.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2
            "#,
        )?;

        let versions = stmt
            .query_map((registry_type_str, package_name), |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        Ok(versions)
    }

    /// Save dist tags for a package
    pub fn save_dist_tags(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        dist_tags: &HashMap<String, String>,
    ) -> Result<(), CacheError> {
        if dist_tags.is_empty() {
            return Ok(());
        }

        let registry_type_str = registry_type.as_str();
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction()?;

        // Get or create package
        let now = Self::current_timestamp_ms();

        tx.execute(
            r#"
            INSERT INTO packages (registry_type, package_name, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(registry_type, package_name) DO NOTHING
            "#,
            (registry_type_str, package_name, now),
        )?;

        let package_id: i64 = tx.query_row(
            "SELECT id FROM packages WHERE registry_type = ?1 AND package_name = ?2",
            (registry_type_str, package_name),
            |row| row.get(0),
        )?;

        // Delete existing dist tags and insert new ones
        tx.execute("DELETE FROM dist_tags WHERE package_id = ?1", [package_id])?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO dist_tags (package_id, tag_name, version) VALUES (?1, ?2, ?3)",
            )?;
            for (tag_name, version) in dist_tags {
                stmt.execute((package_id, tag_name, version))?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Get a specific dist tag for a package
    pub fn get_dist_tag(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        tag_name: &str,
    ) -> Result<Option<String>, CacheError> {
        let registry_type_str = registry_type.as_str();
        let conn = self.lock_conn()?;
        let result = conn.query_row(
            r#"
            SELECT dt.version FROM dist_tags dt
            JOIN packages p ON dt.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2 AND dt.tag_name = ?3
            "#,
            (registry_type_str, package_name, tag_name),
            |row| row.get(0),
        );

        match result {
            Ok(version) => Ok(Some(version)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

impl VersionStorer for Cache {
    fn get_latest_version(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<Option<String>, CacheError> {
        let conn = self.lock_conn()?;

        // First, try to get the "latest" dist-tag (for npm packages)
        let dist_tag_result = conn.query_row(
            r#"
            SELECT dt.version FROM dist_tags dt
            JOIN packages p ON dt.package_id = p.id
            WHERE p.registry_type = ?1 AND p.package_name = ?2 AND dt.tag_name = 'latest'
            "#,
            (registry_type.as_str(), package_name),
            |row| row.get::<_, String>(0),
        );

        if let Ok(version) = dist_tag_result {
            return Ok(Some(version));
        }

        // For registries without dist-tags (GitHub Actions, Go, etc.),
        // find the semantically highest version
        drop(conn); // Release lock before calling get_versions
        let versions = Cache::get_versions(self, registry_type, package_name)?;

        if versions.is_empty() {
            return Ok(None);
        }

        // Find the semantically highest version
        let latest = versions
            .into_iter()
            .filter_map(|v| {
                let parsed = crate::version::semver::parse_version(&v)?;
                // Skip prerelease versions if ignore_prerelease is enabled
                if self.ignore_prerelease && !parsed.pre.is_empty() {
                    return None;
                }
                Some((v, parsed))
            })
            .max_by(|(_, a), (_, b)| a.cmp(b))
            .map(|(v, _)| v);

        Ok(latest)
    }

    fn get_versions(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<Vec<String>, CacheError> {
        Cache::get_versions(self, registry_type, package_name)
    }

    fn version_exists(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        version: &str,
    ) -> Result<bool, CacheError> {
        let registry_type = registry_type.as_str();
        let conn = self.lock_conn()?;
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
        registry_type: RegistryType,
        package_name: &str,
        versions: Vec<String>,
    ) -> Result<(), CacheError> {
        let registry_type = registry_type.as_str();
        debug!(
            "Saving {} versions for {}/{}",
            versions.len(),
            registry_type,
            package_name
        );

        let now = Self::current_timestamp_ms();

        let mut conn = self.lock_conn()?;
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

        // Insert only new versions (skip existing ones)
        // Using INSERT OR IGNORE with UNIQUE constraint on (package_id, version)
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO versions (package_id, version) VALUES (?1, ?2)")?;
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
        let now = Self::current_timestamp_ms();
        let threshold = now - self.refresh_interval;

        let conn = self.lock_conn()?;
        // Exclude packages marked as not found to avoid repeated fetch attempts
        let mut stmt = conn.prepare(
            "SELECT registry_type, package_name FROM packages WHERE updated_at < ?1 AND not_found = 0",
        )?;

        let packages = stmt
            .query_map([threshold], |row| {
                let registry_type_str: String = row.get(0)?;
                let package_name: String = row.get(1)?;
                Ok((registry_type_str, package_name))
            })?
            .filter_map(|result| {
                result.ok().and_then(|(registry_type_str, package_name)| {
                    registry_type_str
                        .parse::<RegistryType>()
                        .ok()
                        .map(|rt| PackageId {
                            registry_type: rt,
                            package_name,
                        })
                })
            })
            .collect();

        Ok(packages)
    }

    fn try_start_fetch(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<bool, CacheError> {
        let registry_type = registry_type.as_str();
        let now = Self::current_timestamp_ms();
        let timeout_threshold = now - FETCH_TIMEOUT_MS;

        let conn = self.lock_conn()?;

        // Try to set fetching_since if:
        // 1. fetching_since is NULL (not being fetched)
        // 2. fetching_since is older than timeout (previous fetch timed out)
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

        // Package might not exist yet - try to insert with fetching_since set
        // INSERT OR IGNORE ensures only the first caller succeeds for new packages
        let rows_inserted = conn.execute(
            r#"
            INSERT OR IGNORE INTO packages (registry_type, package_name, updated_at, fetching_since)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            (registry_type, package_name, now, now),
        )?;

        // Only the first inserter can proceed (rows_inserted > 0)
        // Subsequent callers get rows_inserted = 0 due to UNIQUE constraint
        Ok(rows_inserted > 0)
    }

    fn finish_fetch(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<(), CacheError> {
        let registry_type = registry_type.as_str();
        let conn = self.lock_conn()?;

        conn.execute(
            "UPDATE packages SET fetching_since = NULL WHERE registry_type = ?1 AND package_name = ?2",
            (registry_type, package_name),
        )?;

        Ok(())
    }

    fn get_dist_tag(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        tag_name: &str,
    ) -> Result<Option<String>, CacheError> {
        Cache::get_dist_tag(self, registry_type, package_name, tag_name)
    }

    fn save_dist_tags(
        &self,
        registry_type: RegistryType,
        package_name: &str,
        dist_tags: &HashMap<String, String>,
    ) -> Result<(), CacheError> {
        Cache::save_dist_tags(self, registry_type, package_name, dist_tags)
    }

    fn filter_packages_not_in_cache(
        &self,
        registry_type: RegistryType,
        package_names: &[String],
    ) -> Result<Vec<String>, CacheError> {
        if package_names.is_empty() {
            return Ok(Vec::new());
        }

        let registry_type = registry_type.as_str();
        let conn = self.lock_conn()?;

        // Build WHERE IN clause with placeholders
        let placeholders: Vec<_> = (0..package_names.len())
            .map(|i| format!("?{}", i + 2))
            .collect();
        let placeholders_str = placeholders.join(", ");

        // Consider packages as "cached" if:
        // 1. They have at least one version, OR
        // 2. They are marked as not found (to skip repeated fetch attempts)
        let query = format!(
            r#"
            SELECT p.package_name
            FROM packages p
            WHERE p.registry_type = ?1
              AND p.package_name IN ({})
              AND (EXISTS (SELECT 1 FROM versions v WHERE v.package_id = p.id) OR p.not_found = 1)
            "#,
            placeholders_str
        );

        let mut stmt = conn.prepare(&query)?;

        // Build params: registry_type followed by all package names
        let params: Vec<&dyn rusqlite::ToSql> =
            std::iter::once(&registry_type as &dyn rusqlite::ToSql)
                .chain(package_names.iter().map(|s| s as &dyn rusqlite::ToSql))
                .collect();

        let cached_packages: HashSet<String> = stmt
            .query_map(params.as_slice(), |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Return packages that are NOT in the cache (preserving original order)
        let not_in_cache = package_names
            .iter()
            .filter(|name| !cached_packages.contains(*name))
            .cloned()
            .collect();

        Ok(not_in_cache)
    }

    fn mark_not_found(
        &self,
        registry_type: RegistryType,
        package_name: &str,
    ) -> Result<(), CacheError> {
        let registry_type = registry_type.as_str();
        let conn = self.lock_conn()?;

        conn.execute(
            "UPDATE packages SET not_found = 1 WHERE registry_type = ?1 AND package_name = ?2",
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
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let versions = vec![
            "1.0.0".to_string(),
            "1.1.0".to_string(),
            "2.0.0".to_string(),
        ];
        cache
            .replace_versions(RegistryType::Npm, "axios", versions.clone())
            .unwrap();

        let saved = cache.get_versions(RegistryType::Npm, "axios").unwrap();
        assert_eq!(saved, versions);
    }

    #[test]
    fn replace_versions_updates_existing_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let initial_versions = vec!["1.0.0".to_string()];
        cache
            .replace_versions(RegistryType::Npm, "axios", initial_versions)
            .unwrap();

        let new_versions = vec!["1.0.0".to_string(), "1.1.0".to_string()];
        cache
            .replace_versions(RegistryType::Npm, "axios", new_versions.clone())
            .unwrap();

        let saved = cache.get_versions(RegistryType::Npm, "axios").unwrap();
        assert_eq!(saved, new_versions);
    }

    #[test]
    fn replace_versions_adds_only_new_versions() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Initial versions
        let initial_versions = vec!["1.0.0".to_string(), "1.1.0".to_string()];
        cache
            .replace_versions(RegistryType::Npm, "axios", initial_versions)
            .unwrap();

        // Add mix of existing and new versions
        let updated_versions = vec![
            "1.0.0".to_string(), // existing
            "1.1.0".to_string(), // existing
            "1.2.0".to_string(), // new
            "2.0.0".to_string(), // new
        ];
        cache
            .replace_versions(RegistryType::Npm, "axios", updated_versions)
            .unwrap();

        // Verify all versions are present (no duplicates)
        let mut saved = cache.get_versions(RegistryType::Npm, "axios").unwrap();
        saved.sort();
        assert_eq!(
            saved,
            vec![
                "1.0.0".to_string(),
                "1.1.0".to_string(),
                "1.2.0".to_string(),
                "2.0.0".to_string(),
            ]
        );
    }

    #[test]
    fn get_versions_returns_empty_for_nonexistent_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let versions = cache
            .get_versions(RegistryType::Npm, "nonexistent")
            .unwrap();
        assert!(versions.is_empty());
    }

    #[test]
    fn get_versions_performance_with_1000_versions() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let versions: Vec<String> = (0..1000).map(|i| format!("{}.0.0", i)).collect();
        cache
            .replace_versions(RegistryType::Npm, "large-package", versions.clone())
            .unwrap();

        let start = std::time::Instant::now();
        let retrieved = cache
            .get_versions(RegistryType::Npm, "large-package")
            .unwrap();
        let elapsed = start.elapsed();

        assert_eq!(retrieved.len(), 1000);
        assert!(
            elapsed.as_millis() < 10,
            "get_versions took {}ms, expected < 10ms",
            elapsed.as_millis()
        );
    }

    #[rstest]
    #[case(RegistryType::Npm, "axios", "1.0.0", true)]
    #[case(RegistryType::Npm, "axios", "2.0.0", true)]
    #[case(RegistryType::Npm, "axios", "9.9.9", false)]
    #[case(RegistryType::Npm, "nonexistent", "1.0.0", false)]
    #[case(RegistryType::CratesIo, "axios", "1.0.0", false)]
    fn version_exists_returns_expected(
        #[case] registry_type: RegistryType,
        #[case] package_name: &str,
        #[case] version: &str,
        #[case] expected: bool,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let versions = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        cache
            .replace_versions(RegistryType::Npm, "axios", versions)
            .unwrap();

        assert_eq!(
            cache
                .version_exists(registry_type, package_name, version)
                .unwrap(),
            expected
        );
    }

    #[rstest]
    #[case(RegistryType::Npm, "axios", Some("3.0.0".to_string()))]
    #[case(RegistryType::Npm, "nonexistent", None)]
    #[case(RegistryType::CratesIo, "axios", None)]
    fn get_latest_version_returns_last_inserted(
        #[case] registry_type: RegistryType,
        #[case] package_name: &str,
        #[case] expected: Option<String>,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0".to_string(),
        ];
        cache
            .replace_versions(RegistryType::Npm, "axios", versions)
            .unwrap();

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
        let cache = Cache::new(&db_path, 100, false).unwrap();

        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();
        cache
            .replace_versions(RegistryType::Npm, "lodash", vec!["4.0.0".to_string()])
            .unwrap();

        // Wait for packages to become stale
        std::thread::sleep(std::time::Duration::from_millis(150));

        let stale = cache.get_packages_needing_refresh().unwrap();
        assert_eq!(stale.len(), 2);
        assert!(stale.contains(&PackageId {
            registry_type: RegistryType::Npm,
            package_name: "axios".to_string()
        }));
        assert!(stale.contains(&PackageId {
            registry_type: RegistryType::Npm,
            package_name: "lodash".to_string()
        }));
    }

    #[test]
    fn get_packages_needing_refresh_excludes_fresh_packages() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 1 hour (in ms)
        let cache = Cache::new(&db_path, 3600000, false).unwrap();

        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let stale = cache.get_packages_needing_refresh().unwrap();
        assert!(stale.is_empty());
    }

    #[test]
    fn try_start_fetch_returns_true_for_new_package() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // New package not in DB should allow fetch
        let can_fetch = cache
            .try_start_fetch(RegistryType::Npm, "new-package")
            .unwrap();
        assert!(can_fetch);
    }

    #[test]
    fn try_start_fetch_returns_true_for_package_not_being_fetched() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Pre-populate cache (fetching_since is NULL after replace_versions)
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Package exists but not being fetched should allow fetch
        let can_fetch = cache.try_start_fetch(RegistryType::Npm, "axios").unwrap();
        assert!(can_fetch);
    }

    #[test]
    fn try_start_fetch_returns_false_for_package_being_fetched() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Pre-populate cache
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // First fetch should succeed
        let can_fetch1 = cache.try_start_fetch(RegistryType::Npm, "axios").unwrap();
        assert!(can_fetch1);

        // Second fetch should fail (already being fetched)
        let can_fetch2 = cache.try_start_fetch(RegistryType::Npm, "axios").unwrap();
        assert!(!can_fetch2);
    }

    #[test]
    fn finish_fetch_clears_fetching_state() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Pre-populate cache
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Start fetch
        let can_fetch1 = cache.try_start_fetch(RegistryType::Npm, "axios").unwrap();
        assert!(can_fetch1);

        // Finish fetch
        cache.finish_fetch(RegistryType::Npm, "axios").unwrap();

        // Should be able to fetch again
        let can_fetch2 = cache.try_start_fetch(RegistryType::Npm, "axios").unwrap();
        assert!(can_fetch2);
    }

    #[test]
    fn save_and_get_dist_tags() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let mut dist_tags = std::collections::HashMap::new();
        dist_tags.insert("latest".to_string(), "4.17.21".to_string());
        dist_tags.insert("beta".to_string(), "5.0.0-beta.1".to_string());

        cache
            .save_dist_tags(RegistryType::Npm, "lodash", &dist_tags)
            .unwrap();

        // Get specific dist-tag
        let latest = cache
            .get_dist_tag(RegistryType::Npm, "lodash", "latest")
            .unwrap();
        assert_eq!(latest, Some("4.17.21".to_string()));

        let beta = cache
            .get_dist_tag(RegistryType::Npm, "lodash", "beta")
            .unwrap();
        assert_eq!(beta, Some("5.0.0-beta.1".to_string()));

        // Non-existent tag
        let unknown = cache
            .get_dist_tag(RegistryType::Npm, "lodash", "unknown")
            .unwrap();
        assert_eq!(unknown, None);

        // Non-existent package
        let no_pkg = cache
            .get_dist_tag(RegistryType::Npm, "nonexistent", "latest")
            .unwrap();
        assert_eq!(no_pkg, None);
    }

    #[test]
    fn get_latest_version_prefers_dist_tag_latest_over_last_inserted() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Insert versions in order: stable versions first, then pre-release
        // This simulates npm's time-based ordering where pre-release comes last
        let versions = vec![
            "4.17.20".to_string(),
            "4.17.21".to_string(),               // This is the stable latest
            "0.0.0-insiders.abc123".to_string(), // Pre-release published after stable
        ];
        cache
            .replace_versions(RegistryType::Npm, "tailwindcss", versions)
            .unwrap();

        // Set dist-tags with latest pointing to stable version
        let mut dist_tags = std::collections::HashMap::new();
        dist_tags.insert("latest".to_string(), "4.17.21".to_string());
        dist_tags.insert("insiders".to_string(), "0.0.0-insiders.abc123".to_string());
        cache
            .save_dist_tags(RegistryType::Npm, "tailwindcss", &dist_tags)
            .unwrap();

        // get_latest_version should return dist-tags.latest, not the last inserted version
        let latest = cache
            .get_latest_version(RegistryType::Npm, "tailwindcss")
            .unwrap();
        assert_eq!(latest, Some("4.17.21".to_string()));
    }

    #[test]
    fn get_latest_version_falls_back_to_last_inserted_when_no_dist_tag() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Insert versions without dist-tags (like GitHub Actions)
        let versions = vec!["v3.0.0".to_string(), "v4.0.0".to_string()];
        cache
            .replace_versions(RegistryType::GitHubActions, "actions/checkout", versions)
            .unwrap();

        // No dist-tags set, should return last inserted version
        let latest = cache
            .get_latest_version(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert_eq!(latest, Some("v4.0.0".to_string()));
    }

    #[test]
    fn filter_packages_not_in_cache_returns_only_missing_packages() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Add some packages to cache
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();
        cache
            .replace_versions(RegistryType::Npm, "lodash", vec!["4.0.0".to_string()])
            .unwrap();

        // Query for a mix of cached and uncached packages
        let package_names = vec![
            "axios".to_string(),   // cached
            "lodash".to_string(),  // cached
            "express".to_string(), // NOT cached
            "react".to_string(),   // NOT cached
        ];

        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::Npm, &package_names)
            .unwrap();

        // Should only return packages NOT in cache
        assert_eq!(
            not_in_cache,
            vec!["express".to_string(), "react".to_string()]
        );
    }

    #[test]
    fn filter_packages_not_in_cache_returns_empty_when_all_cached() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let package_names = vec!["axios".to_string()];
        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::Npm, &package_names)
            .unwrap();

        assert!(not_in_cache.is_empty());
    }

    #[test]
    fn filter_packages_not_in_cache_returns_all_when_none_cached() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        let package_names = vec!["express".to_string(), "react".to_string()];
        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::Npm, &package_names)
            .unwrap();

        assert_eq!(
            not_in_cache,
            vec!["express".to_string(), "react".to_string()]
        );
    }

    #[test]
    fn filter_packages_not_in_cache_respects_registry_type() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Add package to npm registry
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Query same package name but different registry
        let package_names = vec!["axios".to_string()];
        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::CratesIo, &package_names)
            .unwrap();

        // Should return axios because it's not in CratesIo registry
        assert_eq!(not_in_cache, vec!["axios".to_string()]);
    }

    #[test]
    fn filter_packages_not_in_cache_treats_zero_versions_as_not_cached() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Simulate a failed fetch: package record exists but no versions
        // This happens when try_start_fetch creates a record but fetch_all_versions fails
        cache
            .try_start_fetch(RegistryType::Npm, "failed-package")
            .unwrap();
        cache
            .finish_fetch(RegistryType::Npm, "failed-package")
            .unwrap();

        // Add a package with versions for comparison
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let package_names = vec!["failed-package".to_string(), "axios".to_string()];
        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::Npm, &package_names)
            .unwrap();

        // Should return failed-package because it has 0 versions
        assert_eq!(not_in_cache, vec!["failed-package".to_string()]);
    }

    #[test]
    fn get_latest_version_filters_prerelease_when_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, true).unwrap(); // ignore_prerelease = true

        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0-alpha".to_string(), // prerelease (highest but filtered)
        ];
        cache
            .replace_versions(RegistryType::GitHubActions, "actions/checkout", versions)
            .unwrap();

        let latest = cache
            .get_latest_version(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert_eq!(latest, Some("2.0.0".to_string())); // highest stable version
    }

    #[test]
    fn get_latest_version_includes_prerelease_when_disabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap(); // ignore_prerelease = false

        let versions = vec![
            "1.0.0".to_string(),
            "2.0.0".to_string(),
            "3.0.0-alpha".to_string(), // prerelease
        ];
        cache
            .replace_versions(RegistryType::GitHubActions, "actions/checkout", versions)
            .unwrap();

        let latest = cache
            .get_latest_version(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert_eq!(latest, Some("3.0.0-alpha".to_string())); // includes prerelease
    }

    #[test]
    fn get_latest_version_returns_none_when_all_versions_are_prerelease_and_filtering_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, true).unwrap(); // ignore_prerelease = true

        let versions = vec!["1.0.0-alpha".to_string(), "1.0.0-beta".to_string()];
        cache
            .replace_versions(RegistryType::GitHubActions, "actions/checkout", versions)
            .unwrap();

        let latest = cache
            .get_latest_version(RegistryType::GitHubActions, "actions/checkout")
            .unwrap();
        assert_eq!(latest, None); // all prerelease, so None
    }

    #[test]
    fn get_latest_version_filters_go_pseudo_version_when_prerelease_filtering_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, true).unwrap(); // ignore_prerelease = true

        let versions = vec![
            "v1.0.0".to_string(),
            "v0.0.0-20210201000000-abc123".to_string(), // pseudo-version (prerelease)
        ];
        cache
            .replace_versions(RegistryType::GoProxy, "github.com/example/module", versions)
            .unwrap();

        let latest = cache
            .get_latest_version(RegistryType::GoProxy, "github.com/example/module")
            .unwrap();
        // pseudo-version is also filtered as prerelease
        assert_eq!(latest, Some("v1.0.0".to_string()));
    }

    #[test]
    fn get_latest_version_filters_go_regular_prerelease_when_enabled() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, true).unwrap(); // ignore_prerelease = true

        let versions = vec![
            "v1.0.0".to_string(),
            "v2.0.0-alpha".to_string(), // regular prerelease
        ];
        cache
            .replace_versions(RegistryType::GoProxy, "github.com/example/module", versions)
            .unwrap();

        let latest = cache
            .get_latest_version(RegistryType::GoProxy, "github.com/example/module")
            .unwrap();
        assert_eq!(latest, Some("v1.0.0".to_string())); // alpha is filtered
    }

    #[test]
    fn mark_not_found_sets_not_found_flag() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Create a package entry via try_start_fetch (simulating a fetch attempt)
        cache
            .try_start_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();

        // Mark as not found
        cache
            .mark_not_found(RegistryType::Npm, "nonexistent")
            .unwrap();

        // Verify the flag is set by checking it's excluded from needing refresh
        cache
            .finish_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let stale = cache.get_packages_needing_refresh().unwrap();
        assert!(
            stale.is_empty(),
            "not_found packages should be excluded from refresh"
        );
    }

    #[test]
    fn get_packages_needing_refresh_excludes_not_found_packages() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        // refresh_interval = 100ms
        let cache = Cache::new(&db_path, 100, false).unwrap();

        // Add a normal package
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        // Add a package and mark as not found
        cache
            .try_start_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();
        cache
            .mark_not_found(RegistryType::Npm, "nonexistent")
            .unwrap();
        cache
            .finish_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();

        // Wait for packages to become stale
        std::thread::sleep(std::time::Duration::from_millis(150));

        let stale = cache.get_packages_needing_refresh().unwrap();

        // Only axios should be returned, not the not_found package
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].package_name, "axios");
    }

    #[test]
    fn filter_packages_not_in_cache_treats_not_found_as_cached() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let cache = Cache::new(&db_path, 86400, false).unwrap();

        // Add a package and mark as not found
        cache
            .try_start_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();
        cache
            .mark_not_found(RegistryType::Npm, "nonexistent")
            .unwrap();
        cache
            .finish_fetch(RegistryType::Npm, "nonexistent")
            .unwrap();

        // Add a normal package
        cache
            .replace_versions(RegistryType::Npm, "axios", vec!["1.0.0".to_string()])
            .unwrap();

        let package_names = vec![
            "nonexistent".to_string(), // marked as not found
            "axios".to_string(),       // has versions
            "express".to_string(),     // truly not in cache
        ];

        let not_in_cache = cache
            .filter_packages_not_in_cache(RegistryType::Npm, &package_names)
            .unwrap();

        // Only express should be returned (nonexistent is not_found, axios has versions)
        assert_eq!(not_in_cache, vec!["express".to_string()]);
    }

    mod migrations {
        use super::*;

        /// Helper to check if a column exists in a table
        fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) > 0 FROM pragma_table_info('{}') WHERE name = '{}'",
                    table, column
                ),
                [],
                |row| row.get(0),
            )
            .unwrap_or(false)
        }

        /// Helper to get user_version
        fn get_user_version(conn: &Connection) -> i32 {
            conn.pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap()
        }

        /// Helper to create initial schema for testing
        fn create_initial_schema(
            conn: &Connection,
            has_fetching_since: bool,
            has_not_found: bool,
            user_version: i32,
        ) {
            let columns = format!(
                "id INTEGER PRIMARY KEY AUTOINCREMENT,
                 registry_type TEXT NOT NULL,
                 package_name TEXT NOT NULL,
                 updated_at INTEGER NOT NULL{}{}",
                if has_fetching_since {
                    ", fetching_since INTEGER"
                } else {
                    ""
                },
                if has_not_found {
                    ", not_found INTEGER NOT NULL DEFAULT 0"
                } else {
                    ""
                }
            );

            conn.execute(
                &format!(
                    "CREATE TABLE packages ({}, UNIQUE(registry_type, package_name))",
                    columns
                ),
                [],
            )
            .unwrap();

            if user_version > 0 {
                conn.pragma_update(None, "user_version", user_version)
                    .unwrap();
            }
        }

        #[rstest]
        // New DB: both columns added
        #[case(false, false, 0, 2)]
        // Existing DB with fetching_since only: not_found added
        #[case(true, false, 0, 2)]
        // Existing DB with both columns: skip (duplicate detection)
        #[case(true, true, 0, 2)]
        // Existing DB with user_version already set: skip migrations
        #[case(true, true, 2, 2)]
        fn migration_applies_correctly(
            #[case] has_fetching_since: bool,
            #[case] has_not_found: bool,
            #[case] initial_version: i32,
            #[case] expected_version: i32,
        ) {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.db");

            // Setup initial schema if not new DB
            if has_fetching_since || has_not_found || initial_version > 0 {
                let conn = Connection::open(&db_path).unwrap();
                create_initial_schema(&conn, has_fetching_since, has_not_found, initial_version);
            }

            // Create cache (triggers migrations)
            let _cache = Cache::new(&db_path, 86400, false).unwrap();

            // Verify final state
            let conn = Connection::open(&db_path).unwrap();
            assert!(
                column_exists(&conn, "packages", "fetching_since"),
                "fetching_since should exist"
            );
            assert!(
                column_exists(&conn, "packages", "not_found"),
                "not_found should exist"
            );
            assert_eq!(get_user_version(&conn), expected_version);
        }

        #[test]
        fn migration_preserves_existing_data() {
            let temp_dir = TempDir::new().unwrap();
            let db_path = temp_dir.path().join("test.db");

            // Create existing DB with data
            {
                let conn = Connection::open(&db_path).unwrap();
                create_initial_schema(&conn, true, false, 0);
                conn.execute(
                    "INSERT INTO packages (registry_type, package_name, updated_at) VALUES ('npm', 'axios', 12345)",
                    [],
                )
                .unwrap();
            }

            // Create cache (triggers migrations)
            let cache = Cache::new(&db_path, 86400, false).unwrap();

            // Verify data is preserved
            let conn = Connection::open(&db_path).unwrap();
            let (name, updated_at): (String, i64) = conn
                .query_row(
                    "SELECT package_name, updated_at FROM packages WHERE registry_type = 'npm'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(name, "axios");
            assert_eq!(updated_at, 12345);

            // Verify cache can read the data
            let versions = cache.get_versions(RegistryType::Npm, "axios").unwrap();
            assert!(versions.is_empty()); // No versions, but package exists
        }
    }
}
