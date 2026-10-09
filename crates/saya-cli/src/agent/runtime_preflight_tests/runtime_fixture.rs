//! The preflight runtime fixture: a unique temp root and a runtime config
//! aimed at the local counting source, backed by a real synthetic SQLite
//! database so the live provider sees real tool definitions and schemas.

use crate::config::runtime::RuntimeConfig;
use saya_config::{AiProvider, ConnectionsFile, ResolvedAi};
use saya_types::DatabaseProfile;
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

pub(in crate::agent::runtime::tests) fn temp_root(label: &str) -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runtime-preflight-{label}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

pub(in crate::agent::runtime::tests) async fn runtime_for_source(
    root: &std::path::Path,
    base_url: String,
    context_window_tokens: Option<u64>,
) -> RuntimeConfig {
    let database = root.join("preflight.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .expect("open synthetic SQLite database");
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite table");
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&seed)
        .await
        .expect("seed synthetic SQLite row");
    seed.close().await;
    let profile = DatabaseProfile::Sqlite {
        path: database.display().to_string(),
        read_only: true,
    };
    let mut runtime = super::super::test_runtime(super::super::default_memory());
    runtime.resolved.profile_name = Some("analytics".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.ai = ResolvedAi {
        provider: AiProvider::OpenaiCompatible,
        model: "preflight-test".into(),
        base_url: Some(base_url),
        api_key: None,
        allow_data_sharing: true,
        temperature: 0.0,
        timeout_seconds: 5,
        idle_timeout_seconds: 5,
        max_output_tokens: 32,
        max_output_tokens_is_default: false,
        context_byte_budget: 256 * 1024,
        context_window_tokens,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: Vec::new(),
    };
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("analytics".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}
