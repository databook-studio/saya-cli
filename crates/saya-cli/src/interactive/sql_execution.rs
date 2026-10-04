//! The immutable source envelope for one direct SQL execution: the result
//! plus where it ran (profile name + opaque identity, connector dialect,
//! issued row cap) and the wall-clock window around the real query.

use crate::config::runtime::RuntimeConfig;
use saya_agent::CancellationToken;
use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector_with_prompt};
use saya_types::{ConnectionError, DatabaseProfile, QueryRequest, QueryResult, SqlDialect};

use super::sql_operation::{SqlOperationError, cancellation};

/// Where one direct SQL execution ran, captured from the SAME resolved
/// profile/connector BEFORE the query runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlSourceBinding {
    pub profile_name: String,
    pub profile_identity: String,
    pub dialect: SqlDialect,
    pub applied_row_cap: usize,
}

/// One direct SQL execution: the result, its source binding, and the
/// wall-clock window (unix millis) around the real query.
#[derive(Debug, Clone)]
pub(crate) struct SqlExecution {
    pub result: QueryResult,
    pub binding: SqlSourceBinding,
    pub started_unix_ms: i64,
    pub finished_unix_ms: i64,
}

impl SqlExecution {
    pub(crate) fn new(
        result: QueryResult,
        binding: SqlSourceBinding,
        started_unix_ms: i64,
        finished_unix_ms: i64,
    ) -> Self {
        Self {
            result,
            binding,
            started_unix_ms,
            finished_unix_ms,
        }
    }

    /// E2-2 builds evidence from the parts; until then this keeps every
    /// field read in all builds.
    pub(crate) fn into_parts(self) -> (QueryResult, SqlSourceBinding, i64, i64) {
        (
            self.result,
            self.binding,
            self.started_unix_ms,
            self.finished_unix_ms,
        )
    }

    pub(crate) fn binding_for(
        profile_name: &str,
        profile_identity: &str,
        dialect: SqlDialect,
        applied_row_cap: usize,
    ) -> SqlSourceBinding {
        SqlSourceBinding {
            profile_name: profile_name.to_owned(),
            profile_identity: profile_identity.to_owned(),
            dialect,
            applied_row_cap,
        }
    }
}

/// Wall clock in unix millis; local so the TUI helper stays its own.
pub(crate) fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Builds and connects with the same options as the cancellation path.
pub(crate) async fn connected_connector(
    runtime: &RuntimeConfig,
    profile: &DatabaseProfile,
    can_prompt: bool,
) -> Result<Box<dyn DatabaseConnector>, SqlOperationError> {
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
    Ok(connector)
}

/// One bounded execution stamped with the window around the real call.
pub(crate) fn request_with_cap(sql: &str, applied_row_cap: usize) -> QueryRequest {
    QueryRequest::new(sql.to_string(), applied_row_cap)
}

/// The cancellation-aware source variant's body: same connector setup as the
/// cancellation path (build + connect, cancelled at both seams), then the
/// envelope stamped around the cancellation-settled execution.
pub(crate) async fn execute_with_cancellation_and_source_inner(
    runtime: &RuntimeConfig,
    profile_name: &str,
    profile: &DatabaseProfile,
    sql: &str,
    can_prompt: bool,
    cancellation: &CancellationToken,
) -> Result<SqlExecution, SqlOperationError> {
    let outer_started = unix_now_ms();
    if cancellation.is_cancelled() {
        return Err(SqlOperationError::Execute(ConnectionError::cancelled()));
    }
    let connector = connected_connector(runtime, profile, can_prompt).await?;
    if cancellation.is_cancelled() {
        return Err(SqlOperationError::Execute(ConnectionError::cancelled()));
    }
    let applied_row_cap = runtime.resolved.max_rows;
    let binding = source_binding(
        runtime,
        profile_name,
        profile,
        connector.as_ref(),
        applied_row_cap,
    );
    let query_started = unix_now_ms();
    let result = cancellation::execute_with_cancellation(
        connector.as_ref(),
        request_with_cap(sql, applied_row_cap),
        cancellation,
    )
    .await
    .map_err(SqlOperationError::Execute)?;
    Ok(SqlExecution::new(
        result,
        binding,
        outer_started.min(query_started),
        unix_now_ms(),
    ))
}

pub(crate) async fn timed_execute(
    connector: &dyn DatabaseConnector,
    sql: &str,
    applied_row_cap: usize,
) -> Result<(QueryResult, i64, i64), SqlOperationError> {
    let started_unix_ms = unix_now_ms();
    let result = connector
        .execute(QueryRequest::new(sql.to_string(), applied_row_cap))
        .await
        .map_err(SqlOperationError::Execute)?;
    Ok((result, started_unix_ms, unix_now_ms()))
}

/// The binding: opaque identity via `profile_identity`, dialect from the
/// live connector, cap = the issued `QueryRequest` cap.
pub(crate) fn source_binding(
    runtime: &RuntimeConfig,
    profile_name: &str,
    profile: &DatabaseProfile,
    connector: &dyn DatabaseConnector,
    applied_row_cap: usize,
) -> SqlSourceBinding {
    let identity =
        crate::profile_identity::profile_identity(profile_name, profile, &runtime.cache_scope);
    SqlExecution::binding_for(
        profile_name,
        identity.as_str(),
        connector.dialect(),
        applied_row_cap,
    )
}

/// Display name: registry hit, else configured selection, else "default".
pub(crate) fn profile_label(runtime: &RuntimeConfig, profile: &DatabaseProfile) -> String {
    runtime
        .connections
        .profiles
        .iter()
        .find(|(_, candidate)| *candidate == profile)
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| {
            runtime
                .resolved
                .profile_name
                .clone()
                .unwrap_or_else(|| "default".to_owned())
        })
}
