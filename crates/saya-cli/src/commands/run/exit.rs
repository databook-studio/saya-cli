//! The one exit-code mapping for every terminal engine outcome, shared by
//! the fresh run and the resume, and the message texts that go with it.
//!
//! Completed 0; paused 6 — the "incomplete but not failed" class the
//! documented scheme lacked, plan G1; cancelled 130; failed by typed cause
//! (safety/query 4, provider 5, connection/config 3). Each outcome says why
//! it stopped: a run never exits silently 0 into an incomplete state, and a
//! failure message names the layer that failed, never the raw payload.

use crate::render::RenderFormat;
use saya_types::{RunFailureCode, RunId};

/// Where a run ended, with its failure code when it failed by cause.
pub(super) struct Settled {
    pub(super) state: saya_harness::engine::RunState,
    pub(super) code: Option<RunFailureCode>,
}

pub(super) fn settle(
    settled: Settled,
    run_id: &RunId,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    use saya_harness::engine::RunState;
    let id = run_id.as_str();
    match settled.state {
        RunState::Completed => Ok(0),
        RunState::Cancelled => {
            crate::commands::output::failure_message(130, format!("run {id} cancelled"), format)
        }
        RunState::Paused => crate::commands::output::failure_message(
            6,
            format!("run {id} paused: resume it with `saya run resume {id}`"),
            format,
        ),
        RunState::Failed => {
            let code = settled.code.unwrap_or(RunFailureCode::Provider);
            crate::commands::output::failure_message(
                failure_code_exit(code),
                format!("run {id} failed: {}", failure_code_cause(code)),
                format,
            )
        }
        state => crate::commands::output::failure_message(
            5,
            format!("run {id} stopped in state {state} without a terminal event"),
            format,
        ),
    }
}

/// A typed terminal cause becomes its message text. The wording's one source
/// is the run renderer (`crate::render_run`) — the exit message and the
/// rendered `Failed` event line must not drift apart.
pub(super) fn failure_code_cause(code: RunFailureCode) -> &'static str {
    crate::render_run::failure_code_cause(code)
}

/// A typed terminal cause's documented exit code.
pub(super) fn failure_code_exit(code: RunFailureCode) -> i32 {
    match code {
        RunFailureCode::SafetyQuery => 4,
        RunFailureCode::ConnectionConfig => 3,
        _ => 5,
    }
}

/// A connection/config failure (class 3): the documented class for anything
/// that stops a run before its first turn without being a plan or step
/// failure.
pub(super) fn connection_failure(
    message: String,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    crate::commands::output::failure_message(3, message, format)
}
