pub(crate) async fn schema(
    connector: &dyn DatabaseConnector,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, ToolError> {
    let started = Instant::now();
    match connector.schema().await {
        Ok(schema) => {
            if let (Some(store), Some(profile_id)) = (store, profile_id) {
                let _ = store.upsert_schema(profile_id, &schema).await;
                audit(
                    store,
                    profile_id,
                    AuditOperation::SchemaRefresh,
                    AuditStatus::Success,
                    started,
                    None,
                    None,
                )
                .await;
            }
            Ok(compact_schema(&schema, connector.dialect()))
        }
        Err(error) => cached(store, profile_id, started, &error, connector.dialect()).await,
    }
}

/// Discovers schema while owning the registry entry until the connector's
/// metadata operation settles. A cancellation never falls back to cached
/// schema: that would turn a cancelled effect into a successful tool result.
pub(crate) async fn schema_with_context(
    operation_entry: &OperationEntry<'_>,
    context: &ToolExecutionContext,
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
) -> Result<serde_json::Value, ToolError> {
    let started = Instant::now();
    let dialect = operation_entry.entry.dialect;
    let (discovery, settled) = match operation::schema(operation_entry, &context.cancellation).await
    {
        operation_metrics::SchemaAttempt::NotStarted => return Err(cancelled_tool_error()),
        operation_metrics::SchemaAttempt::Measured(discovery, settled) => (discovery, settled),
    };
    let sample = |outcome: AgentMetricOutcome| {
        operation_metrics::record_schema_discovery(operation_entry.metrics, discovery, outcome);
    };
    match settled {
        Ok(schema) => {
            sample(AgentMetricOutcome::Succeeded);
            if context.cancellation.is_cancelled() {
                return Err(cancelled_tool_error());
            }
            if let (Some(store), Some(profile_id)) = (store, profile_id) {
                let _ = store.upsert_schema(profile_id, &schema).await;
                audit(
                    store,
                    profile_id,
                    AuditOperation::SchemaRefresh,
                    AuditStatus::Success,
                    started,
                    None,
                    None,
                )
                .await;
            }
            Ok(compact_schema(&schema, dialect))
        }
        Err(operation::OperationError::Connector(error)) => {
            if context.cancellation.is_cancelled() {
                sample(AgentMetricOutcome::Failed);
                Err(cancelled_tool_error())
            } else {
                let fallback = cached(store, profile_id, started, &error, dialect).await;
                sample(if fallback.is_ok() {
                    AgentMetricOutcome::CachedFallback
                } else {
                    AgentMetricOutcome::Failed
                });
                fallback
            }
        }
        Err(operation::OperationError::Cancelled) => {
            sample(AgentMetricOutcome::Cancelled);
            Err(cancelled_tool_error())
        }
        Err(operation::OperationError::TimedOut) => {
            sample(AgentMetricOutcome::Failed);
            Err(cancelled_tool_error())
        }
    }
}

async fn cached(
    store: Option<&SqliteStateStore>,
    profile_id: Option<&str>,
    started: Instant,
    live_error: &saya_types::ConnectionError,
    dialect: SqlDialect,
) -> Result<serde_json::Value, ToolError> {
    let (Some(store), Some(profile_id)) = (store, profile_id) else {
        return Err(ToolError::SchemaDiscoveryFailed(live_error.to_string()));
    };
    match store.get_schema(profile_id).await {
        Ok(Some(cached)) => {
            audit(
                store,
                profile_id,
                AuditOperation::SchemaRefresh,
                AuditStatus::Cached,
                started,
                None,
                None,
            )
            .await;
            let mut value = compact_schema(&cached.schema, dialect);
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "diagnostic".into(),
                    serde_json::Value::String(format!(
                        "using cached schema because live refresh failed: {live_error}"
                    )),
                );
            }
            Ok(value)
        }
        _ => {
            audit(
                store,
                profile_id,
                AuditOperation::SchemaRefresh,
                AuditStatus::Failure,
                started,
                None,
                None,
            )
            .await;
            Err(ToolError::SchemaDiscoveryFailed(live_error.to_string()))
        }
    }
}
