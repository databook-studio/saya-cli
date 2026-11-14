/// (c) A failing live discovery with no cache behind it records exactly one
/// failed SchemaDiscovery sample — never a cached fallback, never a success.
#[tokio::test]
async fn failing_live_schema_without_cache_records_a_failed_discovery_sample() {
    let metrics = Arc::new(LocalMetrics::new());
    let tools = registry_tools(
        Box::new(FailingSchemaConnector),
        None,
        None,
        Some(Arc::clone(&metrics)),
    );
    let result = tools
        .execute_with_context(
            "schema_discovery",
            json!({}),
            tool_context(CancellationToken::new()),
        )
        .await;
    assert!(
        result.is_err(),
        "the failing live discovery is reported: {result:?}"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the lease was acquired before the discovery: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::SchemaDiscovery),
        1,
        "one discovery sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::Failed
        ),
        1,
        "the failed live attempt settles as failed: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::CachedFallback
        ),
        0,
        "no cached fallback without a cache: {snapshot:?}"
    );
}

/// (d) A failing live discovery whose cached schema serves records exactly
/// one SchemaDiscovery sample with the cached-fallback outcome — zero
/// succeeded samples, no second failed sample — and the call still returns
/// the cached schema with its diagnostic.
#[tokio::test]
async fn failing_live_schema_with_cached_fallback_records_one_cached_fallback_sample() {
    let root = temp_root("schema-fallback");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let profile = format!("p-{}", "a".repeat(64));
    store
        .upsert_schema(&profile, &SchemaTree::default())
        .await
        .expect("seed the cached schema");
    let metrics = Arc::new(LocalMetrics::new());
    let tools = registry_tools(
        Box::new(FailingSchemaConnector),
        Some(profile),
        Some(store),
        Some(Arc::clone(&metrics)),
    );
    let result = tools
        .execute_with_context(
            "schema_discovery",
            json!({}),
            tool_context(CancellationToken::new()),
        )
        .await
        .expect("the cached schema serves the call");
    assert!(
        result
            .get("diagnostic")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|diagnostic| diagnostic.contains("using cached schema")),
        "the cached fallback is announced: {result:?}"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::DatabaseWait,
            AgentMetricOutcome::Succeeded
        ),
        1,
        "the lease was acquired before the discovery: {snapshot:?}"
    );
    assert_eq!(
        samples_in(&snapshot, AgentMetricPhase::SchemaDiscovery),
        1,
        "exactly one discovery sample: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::CachedFallback
        ),
        1,
        "the served fallback is the one outcome: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::Succeeded
        ),
        0,
        "the fallback is never a success: {snapshot:?}"
    );
    assert_eq!(
        outcome_in(
            &snapshot,
            AgentMetricPhase::SchemaDiscovery,
            AgentMetricOutcome::Failed
        ),
        0,
        "no second failed sample for the same call: {snapshot:?}"
    );
    let _ = std::fs::remove_dir_all(root);
}
