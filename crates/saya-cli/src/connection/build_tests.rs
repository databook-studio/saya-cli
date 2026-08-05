use super::*;
use saya_types::DatabaseProfile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn unique_temp_db_path(label: &str, index: usize) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-conn-build-test-{}-{}-{}.duckdb",
        label,
        std::process::id(),
        index
    ))
}

fn duckdb_profile(path: &Path) -> DatabaseProfile {
    DatabaseProfile::DuckDb {
        path: path.to_string_lossy().into_owned(),
        read_only: Some(false),
    }
}

#[tokio::test]
async fn build_registry_primary_and_secondary_succeed() {
    let primary_path = unique_temp_db_path("primary", 1);
    let secondary_path = unique_temp_db_path("secondary", 2);

    let primary_prof = duckdb_profile(&primary_path);
    let secondary_prof = duckdb_profile(&secondary_path);

    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let cache_scope = Path::new("/tmp/test_scope");

    let secondaries = vec![("sec_db".to_string(), secondary_prof)];

    let result = build_registry(
        &resolver,
        cache_scope,
        30,
        false,
        "pri_db",
        &primary_prof,
        &secondaries,
    )
    .await;

    assert!(result.is_ok());
    let registry = result.unwrap();

    assert_eq!(registry.names().len(), 2);
    assert!(registry.names().contains(&"pri_db"));
    assert!(registry.names().contains(&"sec_db"));

    let context = registry.describe_context();
    assert!(context.is_some());
    assert!(context.as_ref().unwrap().contains("duckdb"));

    let _ = std::fs::remove_file(&primary_path);
    let _ = std::fs::remove_file(&secondary_path);
}

#[tokio::test]
async fn build_registry_soft_skips_failed_secondary() {
    let primary_path = unique_temp_db_path("primary_soft_skip", 1);
    let primary_prof = duckdb_profile(&primary_path);

    let bad_dir = std::env::temp_dir().join(format!("does-not-exist-{}", std::process::id()));
    let bad_path = bad_dir.join("x.duckdb");
    let bad_secondary_prof = duckdb_profile(&bad_path);

    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let cache_scope = Path::new("/tmp/test_scope");

    let secondaries = vec![("bad_sec".to_string(), bad_secondary_prof)];

    let result = build_registry(
        &resolver,
        cache_scope,
        30,
        false,
        "pri_db",
        &primary_prof,
        &secondaries,
    )
    .await;

    assert!(result.is_ok());
    let registry = result.unwrap();

    assert_eq!(registry.names().len(), 1);
    assert_eq!(registry.names(), vec!["pri_db"]);

    let _ = std::fs::remove_file(&primary_path);
    if bad_dir.exists() {
        let _ = std::fs::remove_dir_all(&bad_dir);
    }
}
