use crate::config::runtime::{RuntimeConfig, RuntimeError};
use crate::interactive::sql_execution::{
    SqlExecution, connected_connector, profile_label, source_binding, timed_execute,
};
use saya_agent::CancellationToken;
use saya_types::{ConnectionError, DatabaseProfile, QueryResult};

#[path = "sql_operation_cancellation.rs"]
pub(crate) mod cancellation;

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
    let name = profile_label(runtime, profile);
    execute_resolved_with_source(runtime, &name, profile, sql, can_prompt)
        .await
        .map(|execution| execution.into_parts().0)
}

/// Executes direct SQL and returns the immutable source envelope: the
/// result plus the binding (profile name/identity, dialect, issued row
/// cap) and the wall-clock window around the real query.
pub(crate) async fn execute_resolved_with_source(
    runtime: &RuntimeConfig,
    profile_name: &str,
    profile: &DatabaseProfile,
    sql: &str,
    can_prompt: bool,
) -> Result<SqlExecution, SqlOperationError> {
    let applied_row_cap = runtime.resolved.max_rows;
    let connector = connected_connector(runtime, profile, can_prompt).await?;
    let binding = source_binding(
        runtime,
        profile_name,
        profile,
        connector.as_ref(),
        applied_row_cap,
    );
    let (result, started_unix_ms, finished_unix_ms) =
        timed_execute(connector.as_ref(), sql, applied_row_cap).await?;
    Ok(SqlExecution::new(
        result,
        binding,
        started_unix_ms,
        finished_unix_ms,
    ))
}

/// Executes bounded direct SQL while keeping cancellation attached to this
/// operation through setup and connector execution. The TUI worker drives
/// the envelope variant directly; this stays as the shared non-envelope
/// entry for callers that only need the result, delegating so both paths
/// run the same connector setup, cancel seams, and safety path.
/// Called from the error-behavior tests (pre-cancelled stop boundary), which
/// link the test target — not the lib — so the lib build sees no caller.
#[cfg(test)]
pub(crate) async fn execute_with_cancellation(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
    cancellation: &CancellationToken,
) -> Result<QueryResult, SqlOperationError> {
    execute_with_cancellation_and_source(runtime, profile_name, sql, can_prompt, cancellation)
        .await
        .map(|execution| execution.into_parts().0)
}

/// Executes bounded direct SQL with cancellation AND returns the immutable
/// source envelope: the result plus the binding (profile name/identity,
/// dialect, issued row cap) and the wall-clock window around the real query.
///
/// The binding and connector reuse the same setup as the non-envelope
/// cancellation path (`connected_connector` = build + connect with the same
/// `ConnectorOptions`); the timestamps wrap the SAME execution (including
/// the cancellation-settled wait — E2-2's Spec Review calls this out).
pub(crate) async fn execute_with_cancellation_and_source(
    runtime: &RuntimeConfig,
    profile_name: Option<&str>,
    sql: &str,
    can_prompt: bool,
    cancellation: &CancellationToken,
) -> Result<SqlExecution, SqlOperationError> {
    let name = profile_name.ok_or(SqlOperationError::NoActiveProfile)?;
    let profile = runtime.named_profile(name)?.clone();
    crate::interactive::sql_execution::execute_with_cancellation_and_source_inner(
        runtime,
        name,
        &profile,
        sql,
        can_prompt,
        cancellation,
    )
    .await
}

/// The retired non-envelope cancellation body: kept as a named marker so the
/// diff shows the old behavior moved into the source variant's inner body
/// (`sql_execution::execute_with_cancellation_and_source_inner`) instead of
/// vanishing. Never called.
#[allow(dead_code)]
async fn execute_resolved_with_cancellation_retired() {}

#[cfg(test)]
#[path = "sql_operation_cancellation_tests.rs"]
mod cancellation_tests;
#[cfg(test)]
#[path = "sql_operation_source_tests.rs"]
mod source_tests;
