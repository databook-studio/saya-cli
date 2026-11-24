//! The production [`CandidateExecutor`] and [`AttemptRunner`] — built over a
//! live connector for the active profile.
//!
//! An attempt is one turn of one chat session: the runner states the session's
//! facts (the folder, the mode, the profiles, the ask's own database budget) and
//! sends the question as that session's one turn, exactly as `saya ask` sends it
//! for a single candidate. The turn's reading is carried onto the question's own
//! stop token, which is what the ask's exit-code table classifies from.

use super::super::decide::CandidateExecutor;
use super::super::profile;
use super::super::question_scope::QuestionScope;
use super::super::turn_config::{AgentRuntimeError, PromptOverrides};
use super::AttemptRunner;
use async_trait::async_trait;
use saya_agent::{AgentEventSink, AgentOutput, ApprovalPolicy, CancellationReason};
use saya_connectors::{ConnectorOptions, DatabaseConnector, build_connector_with_prompt};
use saya_store::SqliteStateStore;
use saya_types::{QueryRequest, QueryResult, SqlDialect};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Builds a connected [`LiveCandidateExecutor`] for the active profile and
/// returns it with the profile's dialect. Mirrors how
/// `commands::connection::build` builds a connector for `query`.
pub(super) async fn build_executor(
    runtime: &crate::config::runtime::RuntimeConfig,
    overrides: &PromptOverrides,
    can_prompt: bool,
    scope: &QuestionScope,
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
            cancellation: scope.cancellation(),
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
pub(super) struct LiveCandidateExecutor {
    connector: Box<dyn DatabaseConnector>,
    max_rows: usize,
    cancellation: saya_agent::CancellationToken,
}

#[async_trait]
impl CandidateExecutor for LiveCandidateExecutor {
    async fn run(&self, sql: &str) -> Option<QueryResult> {
        if self.cancellation.is_cancelled() {
            return None;
        }
        let execution = self
            .connector
            .execute(QueryRequest::new(sql, self.max_rows));
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => (!self.cancellation.is_cancelled()).then(|| result.ok()).flatten(),
            _ = self.cancellation.cancelled() => {
                let (_cancel, _settled) = tokio::join!(self.connector.request_cancel(), execution);
                None
            }
        }
    }
}

/// The production [`AttemptRunner`]: each call opens one session's run, sends
/// the question as its one turn on a fresh history — attempts that share context
/// make the same mistakes, and then agreement between them proves nothing — and
/// closes it.
///
/// A turn's outcome reaches the question's own stop token the way the engines
/// before it stated one: a budget stop latches the dimension it stopped on, the
/// operator's cancel latches the user's stop, and a failure is the run's own
/// error. Everything else (an answer, a settled clarification) is the output's
/// own report.
pub(crate) struct LiveAttemptRunner<'a> {
    runtime: &'a crate::config::runtime::RuntimeConfig,
    prompt: &'a str,
    approval: ApprovalPolicy,
    can_prompt: bool,
    overrides: PromptOverrides,
    /// The run's stream: every attempt's session renders through it, so the
    /// answer goes where the ask's sink goes and the receipts count the calls.
    sink: Arc<dyn AgentEventSink>,
    scope: QuestionScope,
    state_db: Option<SqliteStateStore>,
    /// The folder each attempt's session opens over.
    folder: PathBuf,
    format: crate::render::RenderFormat,
    /// How many attempts this runner started: the next session's job name.
    attempts: AtomicUsize,
}

impl<'a> LiveAttemptRunner<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        runtime: &'a crate::config::runtime::RuntimeConfig,
        prompt: &'a str,
        approval: ApprovalPolicy,
        can_prompt: bool,
        overrides: PromptOverrides,
        sink: Arc<dyn AgentEventSink>,
        scope: QuestionScope,
        state_db: Option<SqliteStateStore>,
        folder: PathBuf,
        format: crate::render::RenderFormat,
    ) -> Self {
        Self {
            runtime,
            prompt,
            approval,
            can_prompt,
            overrides,
            sink,
            scope,
            state_db,
            folder,
            format,
            attempts: AtomicUsize::new(0),
        }
    }

    /// The facts one attempt's session is stated: the same folder, mode and
    /// profiles every attempt gets, under the attempt's own job name, spending
    /// the one-shot ask's database budget.
    fn session(&self) -> crate::interactive::session::line_chat::LineSession {
        let attempt = self.attempts.fetch_add(1, Ordering::Relaxed) + 1;
        crate::interactive::session::line_chat::LineSession {
            session: format!("ask-{}-{attempt}", std::process::id()),
            folder: Some(self.folder.clone()),
            runtime: Arc::new(self.runtime.clone()),
            store: self.store(),
            mode: self.approval,
            overrides: self.overrides.clone(),
            format: self.format,
            sink: Some(Arc::clone(&self.sink)),
            budget: crate::plugins::database::DatabaseBudget::ask(self.runtime),
            can_prompt: self.can_prompt,
            // `saya ask` takes no `--allow`: an attempt starts with no grant.
            grants: saya_agent::SessionGrants::default(),
            // The one-turn composition states the root the attempt records
            // under, so an ask keeps no records inside the folder it asks about.
            state_root: None,
            // The question's own stop: a deadline or a signal latched for this
            // attempt must also end the call waiting at the terminal.
            cancellation: Some(self.scope.cancellation()),
            // An attempt is one turn of a job, not a session: no live task list
            // and none of a session's facts ride it.
            tasks: None,
            facts: None,
            // The attempt says what it was asked and nothing else.
            seed: Vec::new(),
        }
    }

    fn store(&self) -> SqliteStateStore {
        self.state_db
            .clone()
            .unwrap_or_else(|| SqliteStateStore::new(crate::state_path::state_db_path()))
    }
}

#[async_trait]
impl AttemptRunner for LiveAttemptRunner<'_> {
    fn run(&self) -> Pin<Box<dyn Future<Output = Result<AgentOutput, AgentRuntimeError>> + '_>> {
        Box::pin(async move {
            let sent = crate::interactive::session::line_chat::LineChat::one_turn(
                self.session(),
                self.prompt.to_owned(),
            )
            .await
            .map_err(AgentRuntimeError::Agent)?;
            let Some(sent) = sent else {
                return Err(AgentRuntimeError::Agent(
                    "the session refused the question".to_owned(),
                ));
            };
            match &sent.turn.outcome {
                crate::chat::ChatOutcome::Answered => sent.output.ok_or_else(|| {
                    AgentRuntimeError::Agent("the turn ended without a report".to_owned())
                }),
                crate::chat::ChatOutcome::Stopped(budget) => {
                    // The turn's own budget stopped it: the question's stop
                    // token names the dimension, so the ask classifies the same
                    // class and says the same partial sentence.
                    self.scope
                        .cancellation()
                        .cancel_with_reason(CancellationReason::Budget(budget.dimension));
                    sent.output.ok_or_else(|| {
                        AgentRuntimeError::Agent(crate::budget_wording::budget_partial_message(
                            budget.dimension,
                        ))
                    })
                }
                crate::chat::ChatOutcome::Cancelled => {
                    self.scope.cancellation().cancel();
                    sent.output
                        .ok_or_else(|| AgentRuntimeError::Agent("request cancelled".to_owned()))
                }
                crate::chat::ChatOutcome::Failed { message }
                | crate::chat::ChatOutcome::Paused { message } => {
                    Err(AgentRuntimeError::Agent(message.clone()))
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod live_tests;
