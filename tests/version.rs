use tempfile::TempDir;
use version_lsp::version::cache::Cache;

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

    // Save initial versions
    let initial_versions = vec!["1.0.0".to_string()];
    cache
        .replace_versions("npm", "axios", initial_versions)
        .unwrap();

    // Update with new versions
    let new_versions = vec!["1.0.0".to_string(), "1.1.0".to_string()];
    cache
        .replace_versions("npm", "axios", new_versions.clone())
        .unwrap();

    let saved = cache.get_versions("npm", "axios").unwrap();
    assert_eq!(saved, new_versions);
}
