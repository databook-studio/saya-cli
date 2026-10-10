//! RED-first tests for the immutable source envelope (packet E2-1).
//!
//! These tests run against real SQLite through the shared direct SQL
//! operation: the binding must name the SAME resolved profile/connector the
//! query ran on (identity via `profile_identity`, dialect from the
//! connector, applied cap = the row cap in the `QueryRequest`), and the
//! timestamps must bracket the real execution.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use saya_types::{DatabaseProfile, SqlDialect};

use crate::config::runtime::RuntimeConfig;
use crate::interactive::sql_operation;
use crate::profile_identity::profile_identity;

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

async fn seed_five_rows(path: &Path) {
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options)
        .await
        .expect("seed database creates");
    sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("seed table");
    for id in 1..=5 {
        sqlx::query("INSERT INTO t (id) VALUES (?)")
            .bind(id)
            .execute(&pool)
            .await
            .expect("seed row");
    }
    pool.close().await;
}

fn sqlite_runtime(db: &Path, max_rows: usize) -> RuntimeConfig {
    use saya_config::{
        AiProvider, ColorChoice, ConnectionsFile, MemoryMode, OutputFormat, ResolvedAi,
        ResolvedConfig, ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs,
        ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
    };
    let mut connections = ConnectionsFile::default();
    connections.profiles.insert(
        "analytics".to_string(),
        DatabaseProfile::Sqlite {
            path: db.display().to_string(),
            read_only: true,
        },
    );
    RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
            instructions: Default::default(),
            profile_name: Some("analytics".to_string()),
            profile: None,
            investigation_budgets: Default::default(),
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: BTreeMap::new(),
                turns: Some(4),
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: ResolvedRunnerJobs::default(),
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
            ignored_project_overrides: Vec::new(),
            provenance: Default::default(),
            endpoints: BTreeMap::new(),
        },
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-e21-source-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: BTreeMap::new(),
    }
}

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-e21-source-{label}-{}-{}",
        std::process::id(),
        FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("source envelope fixture root");
    root
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[tokio::test]
async fn envelope_binding_names_the_resolved_source_and_cap() {
    let root = fixture_root("binding");
    let db = root.join("source.sqlite3");
    seed_five_rows(&db).await;
    let runtime = sqlite_runtime(&db, 2);
    let profile = runtime.named_profile("analytics").expect("fixture profile");

    let before = unix_now_ms();
    let execution = sql_operation::execute_resolved_with_source(
        &runtime,
        "analytics",
        profile,
        "SELECT * FROM t ORDER BY id",
        false,
    )
    .await
    .expect("the shared operation succeeds");
    let after = unix_now_ms();

    let expected_identity = profile_identity("analytics", profile, &runtime.cache_scope);
    assert_eq!(
        execution.binding.profile_identity,
        expected_identity.as_str(),
        "the binding carries the SAME opaque profile identity",
    );
    assert_eq!(execution.binding.profile_name, "analytics");
    assert_eq!(execution.binding.dialect, SqlDialect::Sqlite);
    assert_eq!(
        execution.binding.applied_row_cap, 2,
        "the binding names the row cap actually issued",
    );
    assert_eq!(execution.result.rows.len(), 2);
    assert!(
        execution.result.truncated,
        "five rows through a cap of two report truncation",
    );
    assert!(execution.started_unix_ms <= execution.finished_unix_ms);
    assert!(execution.started_unix_ms >= before - 1_000);
    assert!(execution.finished_unix_ms <= after + 1_000);

    let plain =
        sql_operation::execute_resolved(&runtime, profile, "SELECT * FROM t ORDER BY id", false)
            .await
            .expect("the legacy entrypoint still succeeds");
    assert_eq!(plain, execution.result);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn same_alias_on_a_different_path_yields_a_different_identity() {
    let root = fixture_root("alias");
    let first_db = root.join("first.sqlite3");
    let second_db = root.join("second.sqlite3");
    seed_five_rows(&first_db).await;
    seed_five_rows(&second_db).await;
    let first = sqlite_runtime(&first_db, 5);
    let second = sqlite_runtime(&second_db, 5);
    let first_profile = first.named_profile("analytics").expect("first profile");
    let second_profile = second.named_profile("analytics").expect("second profile");

    let first_execution = sql_operation::execute_resolved_with_source(
        &first,
        "analytics",
        first_profile,
        "SELECT * FROM t ORDER BY id",
        false,
    )
    .await
    .expect("first database executes");
    let second_execution = sql_operation::execute_resolved_with_source(
        &second,
        "analytics",
        second_profile,
        "SELECT * FROM t ORDER BY id",
        false,
    )
    .await
    .expect("second database executes");

    assert_ne!(
        first_execution.binding.profile_identity, second_execution.binding.profile_identity,
        "the SAME alias on a different sqlite path is a different source",
    );
    let _ = std::fs::remove_dir_all(&root);
}
