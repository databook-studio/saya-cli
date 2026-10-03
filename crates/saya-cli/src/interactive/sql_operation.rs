use crate::config::runtime::{RuntimeConfig, RuntimeError};
use saya_agent::CancellationToken;
use saya_connectors::{ConnectorOptions, build_connector_with_prompt};
use saya_types::{ConnectionError, DatabaseProfile, QueryRequest, QueryResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlOperationPhase {
    Build,
    Connect,
    Execute,
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
    execute_resolved_with_cancellation(runtime, profile, sql, can_prompt, None).await
}

/// Executes bounded direct SQL while keeping cancellation attached to this
/// operation through setup and connector execution.
pub(crate) async fn execute_with_cancellation(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
    cancellation: &CancellationToken,
) -> Result<QueryResult, SqlOperationError> {
    let name = profile_name.ok_or(SqlOperationError::NoActiveProfile)?;
    let profile = runtime.named_profile(name)?;
    execute_resolved_with_cancellation(runtime, profile, sql, can_prompt, Some(cancellation)).await
}

async fn execute_resolved_with_cancellation(
    runtime: &RuntimeConfig,
    profile: &DatabaseProfile,
    sql: &str,
    can_prompt: bool,
    cancellation: Option<&CancellationToken>,
) -> Result<QueryResult, SqlOperationError> {
    if is_cancelled(cancellation) {
        return Err(SqlOperationError::Execute(ConnectionError::cancelled()));
    }
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
    if is_cancelled(cancellation) {
        return Err(SqlOperationError::Execute(ConnectionError::cancelled()));
    }
    connector
        .connect()
        .await
        .map_err(SqlOperationError::Connect)?;
    if is_cancelled(cancellation) {
        return Err(SqlOperationError::Execute(ConnectionError::cancelled()));
    }
    let execution = connector.execute(QueryRequest::new(
        sql.to_string(),
        runtime.resolved.max_rows,
    ));
    tokio::pin!(execution);
    match cancellation {
        None => execution.await.map_err(SqlOperationError::Execute),
        Some(cancellation) => {
            tokio::select! {
                biased;
                result = &mut execution => result.map_err(SqlOperationError::Execute),
                _ = cancellation.cancelled() => {
                    let request = connector.request_cancel();
                    tokio::pin!(request);
                    let (result, _request_outcome) = tokio::join!(&mut execution, &mut request);
                    result.map_err(SqlOperationError::Execute)
                }
            }
        }
    }
}

fn is_cancelled(cancellation: Option<&CancellationToken>) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
}
