//! The production [`CandidateExecutor`] and [`AttemptRunner`] — built over a
//! live connector for the active profile.

use super::super::decide::CandidateExecutor;
use super::super::profile;
use super::super::runtime::{AgentRuntimeError, PromptOverrides, run_prompt_with_sink};
use super::AttemptRunner;
use async_trait::async_trait;
use saya_agent::{AgentOutput, ApprovalPolicy, CancellationToken};
use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector_with_prompt};
use saya_store::SqliteStateStore;
use saya_types::{QueryRequest, QueryResult, SqlDialect};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Builds a connected [`LiveCandidateExecutor`] for the active profile and
/// returns it with the profile's dialect. Mirrors how
/// `commands::connection::build` builds a connector for `query`.
pub(super) async fn build_executor(
    runtime: &crate::config::runtime::RuntimeConfig,
    overrides: &PromptOverrides,
    can_prompt: bool,
) -> Result<(LiveCandidateExecutor, SqlDialect), AgentRuntimeError> {
    let (_name, profile) = profile::selected(runtime, overrides.profile.as_ref())?;
    let Some(profile) = profile else {
        return Err(AgentRuntimeError::Configuration(
            "candidates requires a selected profile".into(),
        ));
    };
    let resolver = runtime.secret_resolver();
    let settings = ConnectorOptions {
        query_timeout_seconds: runtime.resolved.query_timeout_seconds,
        read_only: runtime.resolved.read_only,
        ..Default::default()
    };
    let connector = build_connector_with_prompt(&profile, &resolver, settings, can_prompt)
        .await
        .map_err(|error| AgentRuntimeError::Database(error.to_string()))?;
    connector
        .connect()
        .await
        .map_err(|error| AgentRuntimeError::Database(error.to_string()))?;
    let dialect = connector.dialect();
    Ok((
        LiveCandidateExecutor {
            connector,
            max_rows: runtime.resolved.max_rows,
        },
        dialect,
    ))
}

/// Runs a nominated statement against the active profile's connection.
///
/// Limitation: with `--include-profile`, a statement the agent ran against a
/// *different* profile will fail against this connection and return `None` —
/// that attempt simply does not vote. Safe degradation: a wrong result would be
/// far worse than a missing one, but multi-profile sessions get less from this.
/// Runs a nominated statement against the active profile's connection.
///
/// Limitation: with `--include-profile`, a statement the agent ran against a
/// *different* profile will fail against this connection and return `None` —
/// that attempt simply does not vote. Safe degradation: a wrong result would be
/// far worse than a missing one, but multi-profile sessions get less from this.
pub(super) struct LiveCandidateExecutor {
    connector: Box<dyn DatabaseConnector>,
    max_rows: usize,
}

#[async_trait]
impl CandidateExecutor for LiveCandidateExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        self.connector
            .execute(QueryRequest::new(sql, self.max_rows))
            .await
            .ok()
    }
}

/// The production [`AttemptRunner`]: each call runs one full agent attempt via
/// [`run_prompt_with_sink`] with a fresh empty history — attempts that share
/// context make the same mistakes, and then agreement between them proves
/// nothing.
pub(crate) struct LiveAttemptRunner<'a> {
    runtime: &'a crate::config::runtime::RuntimeConfig,
    prompt: &'a str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    sink: &'a dyn saya_agent::AgentEventSink,
    cancellation: CancellationToken,
    state_db: Option<SqliteStateStore>,
    decider: Option<Arc<dyn saya_agent::ApprovalDecider>>,
    last_sql: Option<String>,
}

impl<'a> LiveAttemptRunner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        runtime: &'a crate::config::runtime::RuntimeConfig,
        prompt: &'a str,
        approval: ApprovalPolicy,
        can_prompt: bool,
        overrides: PromptOverrides,
        sink: &'a dyn saya_agent::AgentEventSink,
        cancellation: CancellationToken,
        state_db: Option<SqliteStateStore>,
        decider: Option<Arc<dyn saya_agent::ApprovalDecider>>,
        last_sql: Option<String>,
    ) -> Self {
        Self {
            runtime,
            prompt,
            approval,
            can_prompt,
            overrides,
            sink,
            cancellation,
            state_db,
            decider,
            last_sql,
        }
    }
}

#[async_trait]
impl AttemptRunner for LiveAttemptRunner<'_> {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>> {
        Box::pin(run_prompt_with_sink(
            self.runtime,
            self.prompt,
            self.approval,
            self.can_prompt,
            self.overrides.clone(),
            Vec::new(),
            self.sink,
            self.cancellation.clone(),
            self.state_db.clone(),
            self.decider.clone(),
            self.last_sql.clone(),
        ))
    }
}
