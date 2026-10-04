//! The headless adapter for the shared automatic-compaction decision: the
//! plain REPL settles a completed turn through the same pure predicate the
//! TUI reads (`auto_compact.rs`), then runs the same compaction operation
//! the manual `/compact` handler runs — one decision, one operation, two
//! surfaces.
//!
//! The numerator is the LAST answering request's input tokens — the last
//! `AgentEvent::Usage { call: UsageCall::Answer }` in the settled output —
//! never the summed `output.usage`, never a failed attempt's report, never
//! the extraction call; no Answer usage event means unknown, and unknown
//! never fires. Automatic compaction is ancillary: its usage is recorded
//! separately (the manual path's learning accounting), a pending
//! clarification or a stop is never authority for it, and a failure latches
//! `auto_compact_failed` until `/compact` or `/clear` re-arms it.

use super::auto_compact::{
    AutoCompactInput, auto_failure_message, auto_success_message, compaction_gates,
    should_auto_compact,
};
use super::compact_task;
use super::session_commands::SessionAction;
use super::session_resume::block_on;
use super::session_runtime::SessionRuntime;
use super::session_state::SessionState;
use crate::config::runtime::RuntimeConfig;
use saya_agent::{AgentEvent, AgentOutput, UsageCall};

/// Builds the shared decision's input from a settled headless turn: mode and
/// window from the runtime exactly as the TUI's settle derives them, the
/// numerator from the output's last answering usage report, the gates from
/// the typed output, and the no-retry latch from the session.
/// `compact_running` is always `false`: headless compaction is synchronous.
pub(crate) fn headless_auto_compact_input(
    runtime: &RuntimeConfig,
    state: &SessionState,
    output: &AgentOutput,
) -> AutoCompactInput {
    let (continued, terminal_stop) = compaction_gates(Some(output));
    AutoCompactInput {
        mode: runtime.resolved.ai.compaction,
        answering_input: last_answering_input(output),
        window: runtime
            .resolved
            .ai
            .context_window_tokens
            .or_else(|| saya_config::context_window_tokens(&state.model)),
        continued,
        terminal_stop,
        compact_running: false,
        auto_failed: state.auto_compact_failed,
    }
}

/// The last answering call's `input_tokens` in the settled output's events:
/// the context numerator, the same figure the TUI captures as
/// `last_answering_input`. The LAST report measures this turn's occupancy
/// (a multi-round turn re-sends the conversation); the summed `output.usage`
/// and a `FailedAttemptUsage` never answer here — absent is not zero.
fn last_answering_input(output: &AgentOutput) -> Option<u64> {
    output.events.iter().rev().find_map(|event| match event {
        AgentEvent::Usage {
            call: UsageCall::Answer,
            usage,
        } => Some(usage.input_tokens),
        _ => None,
    })
}

/// Settles the automatic-compaction decision at the headless turn boundary.
/// Only a turn that completed normally may run automatic work: a pending
/// clarification is not authority for it, and a cancelled or stopped turn
/// never compacts merely because output exists. When the shared predicate
/// fires, the SAME operation the manual `/compact` handler runs executes
/// through the same `block_on` boundary; its usage is recorded separately
/// (learning accounting, never the answering aggregate), and the no-retry
/// latch follows the TUI poll's rule: failure latches `auto_compact_failed`
/// (`compact_task` never sets it; each surface applies the policy), a
/// validated success re-arms it, and a no-work result leaves it alone.
/// The action is returned for the caller to emit beside the turn's own.
pub(crate) fn settle_auto_compact(
    runtime: &RuntimeConfig,
    state: &mut SessionState,
    session: &SessionRuntime,
    output: &AgentOutput,
    clarification: bool,
    cancelled_or_stopped: bool,
) -> Option<SessionAction> {
    if clarification || cancelled_or_stopped {
        return None;
    }
    if !should_auto_compact(&headless_auto_compact_input(runtime, state, output)) {
        return None;
    }
    let outcome = block_on(compact_task::run_with_trigger(
        runtime,
        state,
        &session.universe(),
        true,
    ));
    state.usage.record_learning(outcome.usage);
    if outcome.failed {
        state.auto_compact_failed = true;
        return Some(SessionAction::Error(auto_failure_message(&outcome.message)));
    }
    match outcome.summary {
        Some(summary) => {
            state.auto_compact_failed = false;
            Some(SessionAction::Message(auto_success_message(
                outcome.compacted_turns,
                &summary,
            )))
        }
        // Nothing to compact: not a failure; the trigger stays armed (TUI poll parity).
        None => Some(SessionAction::Message(outcome.message)),
    }
}

#[cfg(test)]
#[path = "session_auto_compact_tests.rs"]
mod session_auto_compact_tests;
