use crate::config::runtime::{RuntimeConfig, RuntimeError};
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::{ConnectionError, QueryRequest, QueryResult};

#[derive(Debug, thiserror::Error)]
pub(crate) enum SqlOperationError {
    #[error("No active profile. Use /connect <profile> first.")]
    NoActiveProfile,
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Connector(#[from] ConnectionError),
}

/// Executes bounded direct SQL for interactive adapters; connectors enforce SQL safety.
pub(crate) async fn execute(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
) -> Result<QueryResult, SqlOperationError> {
    let name = profile_name.ok_or(SqlOperationError::NoActiveProfile)?;
    let profile = runtime.named_profile(name)?;
    let connector = build_connector_with_prompt(
        profile,
        &runtime.secret_resolver(),
        ConnectorOptions {
            query_timeout_seconds: runtime.resolved.query_timeout_seconds,
            read_only: runtime.resolved.read_only,
            ..Default::default()
        },
        can_prompt,
    )
    .await?;
    connector.connect().await?;
    connector
        .execute(QueryRequest::new(
            sql.to_string(),
            runtime.resolved.max_rows,
        ))
        .await
        .map_err(Into::into)
}
