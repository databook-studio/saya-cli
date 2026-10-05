use crate::{
    agent::{
        question_scope::QuestionScope,
        runtime::{AgentRuntimeError, PromptOverrides},
    },
    budget_wording::budget_partial_message,
    config::runtime::RuntimeConfig,
    interactive::sql_operation::{self, SqlOperationPhase},
    local_metrics_format::print_metrics_section,
    metrics_sink::MetricsSink,
    render::{RenderFormat, TerminalEvent},
    stream_render::TerminalSink,
};
use saya_agent::{
    AgentEvent, AgentMode, AgentOutput, ApprovalPolicy, CancellationReason, CancellationToken,
};
use saya_store::{AuditOperation, AuditStatus, SqliteStateStore};
use saya_types::ConnectionError;
use std::{path::PathBuf, time::Instant};

use super::{
    output::{emit, failure, failure_message},
    state,
};

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;

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
    let cancellation = CancellationToken::new();
    let scope = QuestionScope::new(runtime, cancellation.clone());
    // A stop-owner clone: the scope moves into the run, but the Ctrl-C
    // branch must still capture the application-side stop instant.
    let stop_scope = scope.clone();
    // The collector arc is taken before the scope moves into the run: the
    // settled boundary below reads the snapshot the run just finished filling.
    let metrics = scope.metrics();
    let sink = TerminalSink::new(format);
    let sink = MetricsSink::new(&sink, scope.metrics());
    // The one-shot `ask` path composes no session universe at all — writes
    // stay hidden, reads deny — so both capabilities ride the same
    // live-terminal fact: stdin readability and approval obtainability are
    // the same fact here. Unchanged behaviour.
    let work = crate::agent::candidates::run_with_candidates(
        runtime,
        &prompt,
        approval,
        can_prompt,
        can_prompt,
        PromptOverrides {
            included_profiles,
            ..Default::default()
        },
        Vec::new(),
        &sink,
        scope,
        Some(state_db.clone()),
        None,
        None,
        None,
        // The real source arrives with `/mode` in the next slice.
        AgentMode::Build,
        runtime.resolved.candidates,
    );
    tokio::pin!(work);
    let outcome = tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => {
            stop_scope.request_stop();
            let outcome = work.await;
            stop_scope.record_stop_settlement();
            print_metrics_section(&metrics);
            return interrupted_exit(outcome, &cancellation, format);
        }
    };
    print_metrics_section(&metrics);
    settle_ask(outcome, &cancellation, format)
}

/// The exit of an `ask` the user interrupted with Ctrl-C: a plain interrupt
/// (the token carries the user's stop, or no reason) is the quiet 130; a
/// budget or owner-failure stop that latched FIRST keeps its own class and
/// wording through `settle_ask` — the first stop reason decides.
fn interrupted_exit(
    outcome: Result<AgentOutput, AgentRuntimeError>,
    cancellation: &CancellationToken,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match cancellation.reason() {
        None | Some(CancellationReason::User) => Ok(130),
        Some(_) => settle_ask(outcome, cancellation, format),
    }
}

/// The one classification for a settled `ask`, shared by the work-ready and
/// setup-error branches and independent of which `select` branch won: the
/// question token's first stop reason decides — User 130, Budget 6 with
/// truthful partial wording, OwnerFailure 5 — and only a run that stopped for
/// no typed reason falls through to the outcome itself (clarification 6,
/// natural 0, setup error 5). A ready-work/ready-signal race classifies the
/// same either way, because the signal branch latches User before returning.
fn settle_ask(
    outcome: Result<AgentOutput, AgentRuntimeError>,
    cancellation: &CancellationToken,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match cancellation.reason() {
        Some(CancellationReason::User) => {
            failure_message(130, "request cancelled".to_string(), format)
        }
        Some(CancellationReason::Budget(dimension)) => {
            failure_message(6, budget_partial_message(dimension), format)
        }
        Some(CancellationReason::OwnerFailure) => match outcome {
            // The run's own error is the truth the owner failure interrupted;
            // say that rather than a generic line.
            Err(error) => failure_message(5, error.to_string(), format),
            Ok(_) => failure_message(
                5,
                "the run was stopped by an owner failure: its sink could not record answering observations".to_string(),
                format,
            ),
        },
        None => match outcome {
            Ok(output) => Ok(ask_exit_code(&output)),
            Err(error) => failure_message(5, error.to_string(), format),
        },
    }
}

/// The exit code a finished `ask` run returns: 6 — the paused class — when
/// the turn ended on a clarification ("paused, needs input"), 0 when the
/// model answered. The run never fails here: an ask that asked IS a finished
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
