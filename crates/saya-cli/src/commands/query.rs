use crate::{
    agent::runtime::PromptOverrides,
    config::runtime::RuntimeConfig,
    interactive::sql_operation::{self, SqlOperationPhase},
    render::{RenderFormat, TerminalEvent},
    stream_render::TerminalSink,
};
use saya_agent::{AgentEvent, AgentMode, AgentOutput, ApprovalPolicy, CancellationToken};
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
    let sink = TerminalSink::new(format);
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
        cancellation.clone(),
        Some(state_db.clone()),
        None,
        None,
        None,
        // The real source arrives with `/mode` in the next slice.
        AgentMode::Build,
        runtime.resolved.candidates,
    );
    tokio::pin!(work);
    match tokio::select! {
        result = &mut work => result,
        _ = tokio::signal::ctrl_c() => { cancellation.cancel(); return Ok(130); }
    } {
        Ok(output) => Ok(ask_exit_code(&output)),
        Err(error) => failure_message(5, error.to_string(), format),
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
