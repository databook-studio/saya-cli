//! Retry policy against half-written count evidence: the conservative
//! refusal of a pending batch and a missing initialization is pinned, and
//! the operator's explicit Retry lets the bounded step start a fresh
//! attempt from the counts the journal does hold.
//!
//! The battery's family rig — the bounded step and the journaled count
//! row — stays here; its cases live beside it: `refusals` (the
//! conservative policy's pinned refusals), `retry_resumes` (the Retry
//! policy's fresh attempts), and `second_crash` (the closing batch row
//! that keeps a twice-crashed run resumable).

use super::*;
use saya_harness::engine::{AnsweringCountError, IncompletePolicy};
use saya_types::Budgets;

#[path = "retry_counts/refusals.rs"]
mod refusals;

#[path = "retry_counts/retry_resumes.rs"]
mod retry_resumes;

#[path = "retry_counts/second_crash.rs"]
mod second_crash;

fn bounded_step(goal: &str, turns: u64, tool_calls: Option<u64>) -> StepSpec {
    let mut budget = Budgets::default();
    budget.turns = Some(turns);
    budget.tool_calls = tool_calls;
    StepSpec::new(
        goal,
        Capabilities::default(),
        Some(budget),
        Vec::new(),
        None,
    )
    .unwrap()
}

/// One journaled count row, shaped exactly as the engine journals answering
/// observations: every figure the call did not report is absent.
fn count_row(turns: Option<u64>, tool_calls: Option<u64>) -> RunEvent {
    RunEvent::Usage {
        endpoint: "orchestrator".into(),
        tokens: None,
        turns,
        tool_calls,
        cached_input_tokens: None,
        cache_creation_input_tokens: None,
    }
}
