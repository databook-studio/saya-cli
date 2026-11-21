use crate::{
    agent::{
        question_scope::QuestionScope,
        runtime::{AgentRuntimeError, PromptOverrides},
    },
    budget_wording::budget_partial_message,
    config::runtime::RuntimeConfig,
    interactive::sql_operation::{self, SqlOperationPhase},
    render::{RenderFormat, TerminalEvent},
    stream_render::TerminalSink,
};
use saya_agent::{AgentEvent, AgentOutput, ApprovalPolicy, CancellationToken};
use saya_store::{AuditOperation, AuditStatus, SqliteStateStore};
use saya_types::ConnectionError;
use std::{path::PathBuf, sync::Arc, time::Instant};

use super::{
    output::{emit, failure, failure_message},
    state,
};

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

/// The one-shot `ask`, as one turn of one chat session: the question is opened
/// as the session's run, sent as its single turn, and the session closes. What
/// the turn prints is the answer sink's — the same renderer the old turn body
/// printed on, so text and JSON stay the output they were.
///
/// What stays here is the exit-code table below: the session's turn is one
/// answer, and the process says which class it was.
#[allow(clippy::too_many_arguments)]
pub(super) async fn ask(
    prompt: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    approval: ApprovalPolicy,
    can_prompt: bool,
    included_profiles: Vec<String>,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let prompt = super::query_input::input(prompt, file)?;
    if prompt.trim().is_empty() {
        return Err("ask requires a prompt or --file".into());
    }
    // The one surface that reads `[limits.ask]`: a retired table it still
    // honours is said once here, naming what to write instead.
    crate::agent::limits::warn_retired_tables(runtime);
    let cancellation = CancellationToken::new();
    // The question's own scope: the vote's elapsed bound, and the stop the ask
    // classifies from. The turn's database budget is the plugin's — the same
    // `[limits.database]` table, through the ask's override of it.
    let scope = QuestionScope::ask(runtime, cancellation.clone());
    let folder = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let work = crate::agent::candidates::run_with_candidates(
        runtime,
        &prompt,
        approval,
        can_prompt,
        PromptOverrides {
            included_profiles,
            ..Default::default()
        },
        folder,
        format,
        Arc::new(TerminalSink::new(format)),
        scope,
        Some(state_db.clone()),
        runtime.resolved.candidates,
    );
    // The operator's stop, for the phases the turn's own listener cannot reach
    // (the vote's real statements): the question's own token, latched once. A
    // turn in flight hears the signal on its own listener, says nothing, and
    // settles as the cancelled class.
    tokio::pin!(work);
    let settled = tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => {
            cancellation.cancel();
            work.await
        }
    };
    settle_ask(settled, &cancellation, format)
}

/// The one classification for a settled `ask`: the question token's first stop
/// reason decides — User 130 quiet, Budget 6 with truthful partial wording,
/// OwnerFailure 5 — and a run that stopped for no typed reason falls through to
/// the attempt's own turn, as [`turn_exit_code`] reads it.
fn settle_ask(
    outcome: Result<AgentOutput, AgentRuntimeError>,
    cancellation: &CancellationToken,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match cancellation.reason() {
        // The turn said `Request cancelled.` when the operator stopped it; a
        // quiet 130 is the class itself.
        Some(saya_agent::CancellationReason::User) => Ok(130),
        Some(saya_agent::CancellationReason::Budget(dimension)) => {
            failure_message(6, budget_partial_message(dimension), format)
        }
        Some(saya_agent::CancellationReason::OwnerFailure) => match outcome {
            // The run's own error is the truth the owner failure interrupted;
            // say that rather than a generic line.
            Err(error) => failure_message(5, error.to_string(), format),
            Ok(_) => failure_message(
                5,
                "the run was stopped by an owner failure: its sink could not record answering observations".to_string(),
                format,
            ),
        },
        // A future stop reason is still a stop: report the run's own error
        // when it has one, otherwise the unclassified stop, as a failure —
        // never as a clean answer.
        Some(_) => match outcome {
            Err(error) => failure_message(5, error.to_string(), format),
            Ok(_) => failure_message(
                5,
                "the run was stopped for an unclassified reason".to_string(),
                format,
            ),
        },
        None => match outcome {
            Ok(output) => Ok(ask_exit_code(&output)),
            Err(error) => failure_message(5, error.to_string(), format),
        },
    }
}

/// The exit code a finished `ask` run returns: 6 — the paused class — when the
/// turn ended on a clarification ("paused, needs input"), 0 when the model
/// answered. The run never fails here: an ask that asked IS a finished
/// outcome, just not an answered one.
pub(super) fn ask_exit_code(output: &AgentOutput) -> i32 {
    if ended_on_clarification(output) { 6 } else { 0 }
}

/// Whether the run ended on a clarification: the event the loop emitted when
/// the model stopped to ask instead of assuming.
fn ended_on_clarification(output: &AgentOutput) -> bool {
    output
        .events
        .iter()
        .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }))
}

pub(super) async fn run(
    sql: Option<String>,
    file: Option<PathBuf>,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    can_prompt: bool,
    state_db: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let sql = super::query_input::input(sql, file)?;
    if sql.trim().is_empty() {
        return Err("query requires --sql or --file".into());
    }
    let Some(profile) = runtime.resolved.profile.as_ref() else {
        return failure(
            4,
            ConnectionError::invalid_configuration("query requires a selected profile"),
            format,
        );
    };
    let started = Instant::now();
    let profile_name = runtime.resolved.profile_name.as_deref().unwrap_or("none");
    let identity = state::identity(profile_name, profile, &runtime.cache_scope);
    match sql_operation::execute_resolved(runtime, profile, &sql, can_prompt).await {
        Err(error) => {
            state::audit(
                state_db,
                &identity,
                AuditOperation::Query,
                AuditStatus::Failure,
                started.elapsed(),
                None,
                None,
                format,
            )
            .await;
            let code = match error.phase() {
                Some(SqlOperationPhase::Connect) => 3,
                Some(SqlOperationPhase::Build | SqlOperationPhase::Execute) | None => 4,
            };
            failure_message(code, error.to_string(), format)
        }
        Ok(result) => {
            state::audit(
                state_db,
                &identity,
                AuditOperation::Query,
                AuditStatus::Success,
                started.elapsed(),
                Some(result.rows.len()),
                Some(result.truncated),
                format,
            )
            .await;
            emit(TerminalEvent::QueryResult { result }, format);
            Ok(0)
        }
    }
}
