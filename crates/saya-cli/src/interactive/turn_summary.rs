//! The one terminal result a scripted (`--turn-file`) run reports: the
//! turn's status, the process exit code, and an honest tool tally — the
//! calls that completed and the requested-but-unfinished calls whose effects
//! are unknown (a cancelled `workspace_edit` may still have written the file).

use crate::render::TurnStatus;
use saya_agent::AgentEvent;
use std::sync::{Arc, Mutex};

/// One requested-but-unfinished call: the name and, for the file-writing
/// tools, the path the transcript shows — never the call's content.
#[derive(Debug, Clone)]
struct PendingCall {
    name: String,
    path: Option<String>,
}

/// The tool tally one turn's live sink accumulates. Agent events carry no
/// call id, so requests and completions pair by order and name: a completion
/// settles the oldest pending call of the same name; a completion with no
/// pending request is ignored (nothing is vouched for); a denial settles too
/// (a refused call's effect is known: none).
#[derive(Debug, Default, Clone)]
pub(crate) struct ToolTally {
    completed: usize,
    pending: Vec<PendingCall>,
}

/// An entry names at most this many characters, truncated on a char
/// boundary.
const ENTRY_CHARS: usize = 120;

/// At most eight unfinished entries: seven real ones, then `… and N more`.
const UNFINISHED_ENTRIES: usize = 8;

impl ToolTally {
    /// Records one agent event into the tally.
    pub(crate) fn record(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::ToolRequested {
                name, arguments, ..
            } => {
                // Only the file-writing tools carry the path: their
                // cancellation may have landed a write the stream never
                // reported; every other tool's effects are cancelled with it.
                let path = matches!(name.as_str(), "workspace_write" | "workspace_edit")
                    .then(|| crate::agent::tools::tool_call_detail(name, arguments))
                    .flatten();
                self.pending.push(PendingCall {
                    name: name.clone(),
                    path,
                });
            }
            AgentEvent::ToolCompleted { name, .. } => {
                if let Some(index) = self.pending.iter().position(|call| call.name == *name) {
                    self.pending.remove(index);
                    self.completed += 1;
                }
            }
            AgentEvent::ToolDenied { name, .. } => {
                if let Some(index) = self.pending.iter().position(|call| call.name == *name) {
                    self.pending.remove(index);
                }
            }
            _ => {}
        }
    }

    /// The calls that ran to completion.
    pub(crate) fn completed(&self) -> usize {
        self.completed
    }

    /// The requested-never-completed calls: at most [`UNFINISHED_ENTRIES`]
    /// entries of at most [`ENTRY_CHARS`] characters each — seven real ones,
    /// then `… and N more` when the rest overflow.
    pub(crate) fn unfinished_entries(&self) -> Vec<String> {
        let mut entries: Vec<String> = self
            .pending
            .iter()
            .map(|call| match &call.path {
                Some(path) => format!("{} {}", call.name, path),
                None => call.name.clone(),
            })
            .map(|text| bounded(&text, ENTRY_CHARS))
            .collect();
        if entries.len() > UNFINISHED_ENTRIES {
            let more = entries.len() - (UNFINISHED_ENTRIES - 1);
            entries.truncate(UNFINISHED_ENTRIES - 1);
            entries.push(format!("… and {more} more"));
        }
        entries
    }
}

/// Truncates to `max` characters on a char boundary.
fn bounded(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        text.chars().take(max).collect()
    }
}

/// Which prose-turn settlement a turn ended with (the prompt result's
/// variant, without its payloads).
pub(crate) enum ProseOutcome {
    Completed,
    Failed,
    Cancelled,
}

/// The status a prose turn settles to: a budget stop that latched on the
/// question scope reclassifies Failed/Cancelled as `budget_stopped`.
pub(crate) fn prose_status(outcome: ProseOutcome, budget_latched: bool) -> TurnStatus {
    match outcome {
        ProseOutcome::Completed => TurnStatus::Completed,
        ProseOutcome::Failed if budget_latched => TurnStatus::BudgetStopped,
        ProseOutcome::Failed => TurnStatus::Failed,
        ProseOutcome::Cancelled if budget_latched => TurnStatus::BudgetStopped,
        ProseOutcome::Cancelled => TurnStatus::Cancelled,
    }
}

/// What one turn hands the turn-file caller: its status and the tool tally.
pub(crate) struct TurnSummary {
    pub(crate) status: TurnStatus,
    pub(crate) tally: ToolTally,
}

/// The shared tally handle the live sink records into.
pub(crate) type TallyHandle = Arc<Mutex<ToolTally>>;

/// A fresh shared tally.
pub(crate) fn tally_handle() -> TallyHandle {
    Arc::new(Mutex::new(ToolTally::default()))
}

/// The process exit code a terminal status reports: 0 only for a completed
/// turn (an explicit `/exit` included); 5 otherwise, unlike `ask`/`run`.
/// Test seam: production derives the rendered code from the run's own
/// outcome (`emit_turn_result`'s caller computes it once and returns the
/// same value); the unit tests pin that [`final_summary`]'s corrected
/// status always maps back to that same code.
#[cfg(test)]
pub(crate) fn exit_code(status: TurnStatus) -> i32 {
    match status {
        TurnStatus::Completed => 0,
        TurnStatus::Failed | TurnStatus::Cancelled | TurnStatus::BudgetStopped => 5,
    }
}

/// The final summary a scripted run reports: the turn's settled status
/// corrected by the run's own outcome, so the rendered `exit_code` equals
/// the process exit code for every combination. A completed settlement whose
/// run ended errored — the per-turn save refused at the record cap —
/// reclassifies as `failed` (exit 5), never the completed/0 the process
/// contradicts; a failed, cancelled, or budget-stopped settlement keeps its
/// status when the run errored; a run that did not error reports completed,
/// the only status whose exit code is 0. The tool tally always passes
/// through untouched.
pub(crate) fn final_summary(summary: TurnSummary, run_errored: bool) -> TurnSummary {
    let status = match (summary.status, run_errored) {
        (TurnStatus::Completed, true) => TurnStatus::Failed,
        (status, true) => status,
        (_, false) => TurnStatus::Completed,
    };
    TurnSummary {
        status,
        tally: summary.tally,
    }
}

#[cfg(test)]
#[path = "turn_summary_tests.rs"]
mod tests;
