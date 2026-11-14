//! DatabaseConnect phase samples recorded around `build_registry`'s eager
//! connects, read through a real `LocalMetrics` collector and real SQLite
//! connectors.

use super::build_registry;
use crate::local_metrics::{LocalMetrics, outcome_index, phase_index};
use saya_agent::{AgentMetricOutcome, AgentMetricPhase};
use saya_types::DatabaseProfile;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn sqlite_profile(path: &Path) -> DatabaseProfile {
    DatabaseProfile::Sqlite {
        path: path.to_string_lossy().into_owned(),
        read_only: false,
    }
}

async fn sqlite_file(label: &str, index: usize) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "saya-conn-metrics-{label}-{:?}-{index}.sqlite3",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(options)
        .await
        .expect("fixture sqlite file exists");
    sqlx::query("CREATE TABLE saya_metrics_fixture (id INTEGER)")
        .execute(&pool)
        .await
        .expect("fixture table written");
    pool.close().await;
    path
}

fn db_connect_cell(metrics: &LocalMetrics) -> crate::local_metrics::PhaseCell {
    let snapshot = metrics.snapshot();
    snapshot.phases[phase_index(AgentMetricPhase::DatabaseConnect).expect("known phase indexed")]
}

#[tokio::test]
async fn a_real_sqlite_primary_records_one_succeeded_database_connect_sample() {
    let path = sqlite_file("primary", 1).await;
    let profile = sqlite_profile(&path);
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let metrics = LocalMetrics::new();

    let result = build_registry(
        (&resolver, Path::new("/tmp/test_scope")),
        30,
        false,
        "pri_db",
        &profile,
        &[],
        Some(&metrics),
    )
    .await;

    let (registry, failures) = result.expect("primary connects");
    assert!(failures.is_empty());
    assert_eq!(registry.names(), vec!["pri_db"]);
    let cell = db_connect_cell(&metrics);
    assert_eq!(cell.samples, 1, "one connect, one sample: {cell:?}");
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Succeeded).expect("known outcome")],
        1,
        "the connect succeeded: {cell:?}"
    );
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Failed).expect("known outcome")],
        0
    );
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn a_real_secondary_adds_a_second_succeeded_database_connect_sample() {
    let primary_path = sqlite_file("pair_primary", 2).await;
    let secondary_path = sqlite_file("pair_secondary", 3).await;
    let primary = sqlite_profile(&primary_path);
    let secondary = sqlite_profile(&secondary_path);
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let metrics = LocalMetrics::new();

    let result = build_registry(
        (&resolver, Path::new("/tmp/test_scope")),
        30,
        false,
        "pri_db",
        &primary,
        &[("sec_db".to_string(), secondary)],
        Some(&metrics),
    )
    .await;

    let (registry, failures) = result.expect("both connect");
    assert!(failures.is_empty());
    assert_eq!(registry.names().len(), 2);
    let cell = db_connect_cell(&metrics);
    assert_eq!(cell.samples, 2, "primary and secondary sampled: {cell:?}");
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Succeeded).expect("known outcome")],
        2,
        "both connects succeeded: {cell:?}"
    );
    let _ = std::fs::remove_file(&primary_path);
    let _ = std::fs::remove_file(&secondary_path);
}

#[tokio::test]
async fn a_secondary_that_cannot_open_tolerates_failure_without_a_connect_sample() {
    let primary_path = sqlite_file("orphan_primary", 4).await;
    let orphan = std::env::temp_dir()
        .join(format!(
            "saya-conn-metrics-missing-{:?}",
            std::process::id()
        ))
        .join("x.sqlite3");
    let primary = sqlite_profile(&primary_path);
    let orphan_profile = sqlite_profile(&orphan);
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let metrics = LocalMetrics::new();

    let result = build_registry(
        (&resolver, Path::new("/tmp/test_scope")),
        1,
        false,
        "pri_db",
        &primary,
        &[("orphan_sec".to_string(), orphan_profile)],
        Some(&metrics),
    )
    .await;

    let (registry, failures) = result.expect("the registry still builds");
    assert_eq!(registry.names(), vec!["pri_db"]);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "orphan_sec");
    assert!(!failures[0].1.is_empty());
    let cell = db_connect_cell(&metrics);
    assert_eq!(
        cell.samples, 1,
        "only the primary's actual connect is sampled; construction is not: {cell:?}"
    );
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Succeeded).expect("known outcome")],
        1,
        "the primary's connect succeeded: {cell:?}"
    );
    assert_eq!(
        cell.outcomes[outcome_index(AgentMetricOutcome::Failed).expect("known outcome")],
        0,
        "no failed connect sample for a build-stage failure: {cell:?}"
    );
    let _ = std::fs::remove_file(&primary_path);
}

#[tokio::test]
async fn metrics_none_records_nothing_and_leaves_the_registry_equal() {
    let primary_path = sqlite_file("quiet_primary", 6).await;
    let secondary_path = sqlite_file("quiet_secondary", 7).await;
    let primary = sqlite_profile(&primary_path);
    let secondary = sqlite_profile(&secondary_path);
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());

    let result = build_registry(
        (&resolver, Path::new("/tmp/test_scope")),
        30,
        false,
        "pri_db",
        &primary,
        &[("sec_db".to_string(), secondary)],
        None,
    )
    .await;

    let (registry, failures) = result.expect("registry builds without metrics");
    assert!(failures.is_empty());
    assert_eq!(registry.names().len(), 2);
    assert!(registry.names().contains(&"pri_db"));
    assert!(registry.names().contains(&"sec_db"));
    let _ = std::fs::remove_file(&primary_path);
    let _ = std::fs::remove_file(&secondary_path);
}
