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
    let (registry, failures) = result.unwrap();

    assert!(failures.is_empty());
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
    let (registry, failures) = result.unwrap();

    assert_eq!(registry.names().len(), 1);
    assert_eq!(registry.names(), vec!["pri_db"]);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "bad_sec");
    assert!(!failures[0].1.is_empty());

    let _ = std::fs::remove_file(&primary_path);
    if bad_dir.exists() {
        let _ = std::fs::remove_dir_all(&bad_dir);
    }
}

#[tokio::test]
async fn build_registry_multiple_good_secondaries_connect() {
    let primary_path = unique_temp_db_path("primary_multi", 1);
    let sec1_path = unique_temp_db_path("secondary_multi", 2);
    let sec2_path = unique_temp_db_path("secondary_multi", 3);
    let sec3_path = unique_temp_db_path("secondary_multi", 4);

    let primary_prof = duckdb_profile(&primary_path);
    let sec1_prof = duckdb_profile(&sec1_path);
    let sec2_prof = duckdb_profile(&sec2_path);
    let sec3_prof = duckdb_profile(&sec3_path);

    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let cache_scope = Path::new("/tmp/test_scope");

    let secondaries = vec![
        ("sec1".to_string(), sec1_prof),
        ("sec2".to_string(), sec2_prof),
        ("sec3".to_string(), sec3_prof),
    ];

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
    let (registry, failures) = result.unwrap();

    assert!(failures.is_empty());
    assert_eq!(registry.names().len(), 4);
    assert!(registry.names().contains(&"pri_db"));
    assert!(registry.names().contains(&"sec1"));
    assert!(registry.names().contains(&"sec2"));
    assert!(registry.names().contains(&"sec3"));

    let _ = std::fs::remove_file(&primary_path);
    let _ = std::fs::remove_file(&sec1_path);
    let _ = std::fs::remove_file(&sec2_path);
    let _ = std::fs::remove_file(&sec3_path);
}

#[tokio::test]
async fn build_registry_bad_secondary_does_not_abort_good_secondaries() {
    let primary_path = unique_temp_db_path("primary_mixed", 1);
    let sec1_path = unique_temp_db_path("secondary_mixed", 2);
    let sec2_path = unique_temp_db_path("secondary_mixed", 3);

    let bad_dir = std::env::temp_dir().join(format!("does-not-exist-mixed-{}", std::process::id()));
    let bad_path = bad_dir.join("bad.duckdb");

    let primary_prof = duckdb_profile(&primary_path);
    let sec1_prof = duckdb_profile(&sec1_path);
    let bad_prof = duckdb_profile(&bad_path);
    let sec2_prof = duckdb_profile(&sec2_path);

    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let cache_scope = Path::new("/tmp/test_scope");

    let secondaries = vec![
        ("sec1".to_string(), sec1_prof),
        ("bad_sec".to_string(), bad_prof),
        ("sec2".to_string(), sec2_prof),
    ];

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
    let (registry, failures) = result.unwrap();

    assert_eq!(registry.names().len(), 3);
    assert!(registry.names().contains(&"pri_db"));
    assert!(registry.names().contains(&"sec1"));
    assert!(registry.names().contains(&"sec2"));

    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "bad_sec");
    assert!(!failures[0].1.is_empty());

    let _ = std::fs::remove_file(&primary_path);
    let _ = std::fs::remove_file(&sec1_path);
    let _ = std::fs::remove_file(&sec2_path);
    if bad_dir.exists() {
        let _ = std::fs::remove_dir_all(&bad_dir);
    }
}
