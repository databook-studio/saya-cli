//! The answering-count reconciliation: the crash's half-written count
//! evidence for the first incomplete step, settled under the operator's
//! incomplete-step policy before the run continues.

use saya_agent::AnsweringObservation;
use saya_types::{Budgets, RunEvent};

use crate::engine::AnsweringCountError;
use crate::engine::sink::EngineEventSink;
use crate::engine::usage::StepAnsweringCounts;
use crate::journal::Journal;

use super::contract::{IncompletePolicy, ResumeError};

/// Reconciles the first incomplete step's answering-count evidence and
/// returns the sink carrying the settled counts.
pub(super) fn reconcile(
    journal: &Journal,
    mut sink: EngineEventSink,
    step: usize,
    budget: &Budgets,
    replayed_counts: Option<Result<StepAnsweringCounts, AnsweringCountError>>,
    incomplete_policy: IncompletePolicy,
) -> Result<EngineEventSink, ResumeError> {
    let mut counts = match replayed_counts {
        Some(Ok(counts)) => counts,
        Some(Err(source)) => return Err(ResumeError::CountEvidence { step, source }),
        None => StepAnsweringCounts::default(),
    };
    if let Err(source) = counts.remaining(step, budget.turns, budget.tool_calls) {
        match source {
            AnsweringCountError::PendingBatch | AnsweringCountError::MissingInitialization
                if incomplete_policy == IncompletePolicy::Retry => {}
            _ => return Err(ResumeError::CountEvidence { step, source }),
        }
        // Retry reconciles the crash's half-written evidence: a pending
        // request stays charged, its unobserved batch settling to zero —
        // the operator's accepted risk; missing initialization charged
        // nothing, so the fresh attempt restarts from fresh counts. The
        // settlement is journaled, not only counted in memory: the live
        // settle's closing row is what keeps a second crash after the
        // resumed attempt's own admission replaying as admit, batch,
        // admit instead of two admissions in a row, which
        // `from_journal` rejects under every policy.
        if source == AnsweringCountError::PendingBatch {
            journal
                .append(&RunEvent::Usage {
                    endpoint: crate::endpoints::ORCHESTRATOR_ROLE.to_string(),
                    tokens: None,
                    turns: None,
                    tool_calls: Some(0),
                    cached_input_tokens: None,
                    cache_creation_input_tokens: None,
                })
                .map_err(|source| ResumeError::Journal { source })?;
            let batch = AnsweringObservation::ToolBatchObserved { requested: 0 };
            counts
                .observe(batch)
                .map_err(|source| ResumeError::CountEvidence { step, source })?;
        } else {
            counts = StepAnsweringCounts::default();
        }
        sink = sink.with_answering_counts(counts);
    }
    Ok(sink)
}
