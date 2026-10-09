/// One-connection tools around an arbitrary connector, with the profile
/// identity and state store the schema tool's cached-fallback path reads.
fn registry_tools(
    connector: Box<dyn DatabaseConnector>,
    profile_id: Option<String>,
    store: Option<SqliteStateStore>,
    metrics: Option<Arc<LocalMetrics>>,
) -> DatabaseTools {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector,
            dialect: SqlDialect::Sqlite,
            profile_id,
        },
    );
    DatabaseTools::with_registry(registry, 100, true, store).with_metrics(metrics)
}

fn tool_context(cancellation: CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: 1024,
        cancellation,
    }
}
