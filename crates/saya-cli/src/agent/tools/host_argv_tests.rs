//! Host-lane argv admission, asserted before cloning and before
//! journalling: count, per-item bytes, and aggregate bytes refuse with the
//! same numbers the sandbox lane enforces.

use super::host_argv::validate_host_json_argv;
use saya_harness::runner::{MAX_ARG_BYTES, MAX_ARG_COUNT, MAX_ARGV_BYTES};

#[test]
fn host_argv_count_item_and_total_bounds_refuse_before_cloning() {
    let too_many = vec![serde_json::json!(""); MAX_ARG_COUNT + 1];
    let error = validate_host_json_argv(&too_many)
        .expect_err("one argument past the count bound must refuse");
    assert!(
        error.to_string().contains("too many arguments"),
        "the refusal names the count bound: {error}"
    );

    let too_long = vec![serde_json::json!("x".repeat(MAX_ARG_BYTES + 1))];
    let error = validate_host_json_argv(&too_long)
        .expect_err("one byte past the per-item bound must refuse");
    assert!(
        error.to_string().contains("per-argument byte limit"),
        "the refusal names the per-item bound: {error}"
    );

    let item = "x".repeat(MAX_ARG_BYTES);
    let too_wide: Vec<serde_json::Value> = (0..(MAX_ARGV_BYTES / MAX_ARG_BYTES + 1))
        .map(|_| serde_json::json!(item.clone()))
        .collect();
    let error = validate_host_json_argv(&too_wide)
        .expect_err("one item past the aggregate bound must refuse");
    assert!(
        error.to_string().contains("aggregate byte limit"),
        "the refusal names the aggregate bound: {error}"
    );
}

/// The composed `run_command` executor refuses oversized argv at parse time:
/// no child spawns, no journal line lands, and the error names the bound.
/// The lane has no staging directory here, so the config points at an empty
/// PATH — resolution would fail anyway, but the argv refusal fires first.
#[tokio::test]
async fn host_lane_refuses_oversized_argv_before_spawn_and_journal() {
    use saya_agent::ToolExecutor;

    let database = crate::agent::tools::DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let database = std::sync::Arc::new(database);
    let config = saya_harness::host::HostConfig::new(
        "/nonexistent-path-for-host-argv-admission",
        std::env::temp_dir(),
        std::time::Duration::from_secs(600),
    )
    .expect("the host config builds");
    let journal_dir =
        std::env::temp_dir().join(format!("saya-host-argv-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&journal_dir);
    std::fs::create_dir_all(&journal_dir).expect("journal dir must create");
    let journal = std::sync::Arc::new(saya_store::SessionJournal::open(&journal_dir));
    let tools = super::run_tools::RunTools::compose(database, None, None, None)
        .with_session_journal(journal.clone())
        .with_host(config, &saya_agent::CancellationToken::new());

    for args in [
        vec![serde_json::json!(""); MAX_ARG_COUNT + 1],
        vec![serde_json::json!("x".repeat(MAX_ARG_BYTES + 1))],
    ] {
        let arguments = serde_json::json!({"program": "probe", "args": args});
        let error = tools
            .execute("run_command", arguments)
            .await
            .expect_err("oversized host argv must refuse");
        assert!(
            error.to_string().contains("argument"),
            "the refusal names the argv bound: {error}"
        );
    }
    assert!(
        !journal_dir.join("journal.ndjson").exists(),
        "an argv refusal before journalling leaves no journal line"
    );
    let _ = std::fs::remove_dir_all(&journal_dir);
}

#[test]
fn host_argv_at_each_bound_is_accepted() {
    let at_count = vec![serde_json::json!(""); MAX_ARG_COUNT];
    validate_host_json_argv(&at_count).expect("exactly the count bound passes");

    let at_item = vec![serde_json::json!("x".repeat(MAX_ARG_BYTES))];
    validate_host_json_argv(&at_item).expect("exactly the per-item bound passes");

    let item = "x".repeat(MAX_ARG_BYTES);
    let at_total: Vec<serde_json::Value> = (0..(MAX_ARGV_BYTES / MAX_ARG_BYTES))
        .map(|_| serde_json::json!(item.clone()))
        .collect();
    validate_host_json_argv(&at_total).expect("exactly the aggregate bound passes");
}
