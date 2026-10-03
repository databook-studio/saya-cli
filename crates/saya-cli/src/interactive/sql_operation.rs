use crate::config::runtime::{RuntimeConfig, RuntimeError};
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::{ConnectionError, DatabaseProfile, QueryRequest, QueryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlOperationPhase {
    Build,
    Connect,
    Execute,
}

impl SqlOperationPhase {
    pub(crate) const fn exit_code(self) -> i32 {
        match self {
            Self::Connect => 3,
            Self::Build | Self::Execute => 4,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SqlOperationError {
    #[error("No active profile. Use /connect <profile> first.")]
    NoActiveProfile,
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error("{0}")]
    Build(#[source] ConnectionError),
    #[error("{0}")]
    Connect(#[source] ConnectionError),
    #[error("{0}")]
    Execute(#[source] ConnectionError),
}

impl SqlOperationError {
    pub(crate) const fn phase(&self) -> Option<SqlOperationPhase> {
        match self {
            Self::NoActiveProfile => None,
            Self::Runtime(_) | Self::Build(_) => Some(SqlOperationPhase::Build),
            Self::Connect(_) => Some(SqlOperationPhase::Connect),
            Self::Execute(_) => Some(SqlOperationPhase::Execute),
        }
    }
}

#[cfg(test)]
#[path = "sql_operation_tests.rs"]
mod tests;

/// Executes bounded direct SQL for interactive adapters; connectors enforce SQL safety.
pub(crate) async fn execute(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
) -> Result<QueryResult, SqlOperationError> {
    let name = profile_name.ok_or(SqlOperationError::NoActiveProfile)?;
    let profile = runtime.named_profile(name)?;
    execute_resolved(runtime, profile, sql, can_prompt).await
}

/// Executes direct SQL for an adapter that already resolved its profile.
pub(crate) async fn execute_resolved(
    runtime: &RuntimeConfig,
    profile: &DatabaseProfile,
    sql: &str,
    can_prompt: bool,
) -> Result<QueryResult, SqlOperationError> {
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
    .await
    .map_err(SqlOperationError::Build)?;
    connector
        .connect()
        .await
        .map_err(SqlOperationError::Connect)?;
    connector
        .execute(QueryRequest::new(
            sql.to_string(),
            runtime.resolved.max_rows,
        ))
        .await
        .map_err(SqlOperationError::Execute)
}
