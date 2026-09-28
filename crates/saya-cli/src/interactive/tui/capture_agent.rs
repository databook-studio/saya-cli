//! The agent-capture pairing (D12/C2): the bounded queue of unmatched
//! capture outcomes for the current turn, and the pairing of a promoted
//! query with its capture. Split from `capture.rs` (the direct-`/sql`
//! capture and the evidence constructors) by concern, to keep both files
//! under the size cap.
//!
//! Captures arrive on the turn's stream channel before their completion and
//! queue until a successful completion promotes a pending query; the FIRST
//! queued outcome whose (sql, connection) equals the promoted query's is
//! taken. Pairing is by content, never position: a reordered, refused, or
//! missing capture can only leave the slot empty — the snapshot then offers
//! `--refresh` — never an older or foreign result; no silent fallback.

use std::collections::VecDeque;

use super::capture::{CAPTURE_BUDGET_BYTES, CapturedResult, agent_evidence, human_bytes};
use super::types::PendingQuery;
use crate::agent::tools::{AgentCapture, CaptureRefusalReason};
use saya_types::ExecutionEvidence;

/// The bound on capture outcomes queued unmatched for one turn.
pub(crate) const MAX_UNMATCHED_CAPTURES: usize = 8;

/// One unmatched capture outcome queued for the current turn: the typed
/// result of a successful query, or the refusal with its reason (nothing
/// held).
pub(crate) enum AgentCaptureOutcome {
    Captured(AgentCapture),
    Refused {
        sql: String,
        connection: String,
        reason: CaptureRefusalReason,
    },
}

impl AgentCaptureOutcome {
    /// Whether this outcome belongs to the query named by (sql, connection).
    fn matches(&self, sql: &str, connection: &str) -> bool {
        match self {
            Self::Captured(capture) => capture.sql == sql && capture.connection == connection,
            Self::Refused {
                sql: r_sql,
                connection: r_conn,
                ..
            } => r_sql == sql && r_conn == connection,
        }
    }
}

/// Why the latest promoted agent query's rows are not held — the reason the
/// snapshot's refusal message names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureGap {
    /// The result was over the accounted capture budget: refused.
    OverBudget,
    /// The model's message was truncated to the conversation byte budget: it
    /// saw a prefix, so the full result is not its evidence (R3).
    ModelViewTruncated,
    /// Redaction replaced secret-shaped material before the message reached
    /// the model: it never saw those values (R3).
    ModelViewRedacted,
    /// No capture arrived for the promoted query.
    Missing,
}

impl CaptureGap {
    /// The refusal message the snapshot shows when this gap is why the latest
    /// promoted agent query's rows are not held: the budget size for an
    /// over-budget refusal, the model-view reason for a truncated or redacted
    /// model view (R3 decision 3), and no invented reason for a missing
    /// capture.
    pub(crate) fn message(self) -> String {
        match self {
            Self::OverBudget => format!(
                "The latest query's rows were not captured (larger than {}). \
                 Use /export --refresh to re-run it.",
                human_bytes(CAPTURE_BUDGET_BYTES)
            ),
            Self::ModelViewTruncated => {
                "The agent received a truncated version of this result, so it was \
                 not captured. Use /export --refresh to re-run it."
                    .to_owned()
            }
            Self::ModelViewRedacted => {
                "The agent received a redacted version of this result, so it was \
                 not captured. Use /export --refresh to re-run it."
                    .to_owned()
            }
            Self::Missing => "The latest query's rows were not captured. \
                 Use /export --refresh to re-run it."
                .to_owned(),
        }
    }
}

/// The turn's unmatched capture outcomes, plus the gap reason the snapshot
/// reads when the latest promoted agent query's rows are not held. The queue
/// is cleared at turn end and on FIFO desync; the gap survives the turn — it
/// names the latest promoted query's outcome, which a later snapshot may
/// still be asked about.
pub(crate) struct AgentCaptures {
    unmatched: VecDeque<AgentCaptureOutcome>,
    pub(crate) gap: Option<CaptureGap>,
}

impl AgentCaptures {
    pub(crate) const fn new() -> Self {
        Self {
            unmatched: VecDeque::new(),
            gap: None,
        }
    }

    /// Queues one outcome, bounded at [`MAX_UNMATCHED_CAPTURES`]: beyond it
    /// the oldest is dropped — safe under content pairing, where that can
    /// only cause a later no-match, never attach a foreign result.
    pub(crate) fn push(&mut self, outcome: AgentCaptureOutcome) {
        if self.unmatched.len() >= MAX_UNMATCHED_CAPTURES {
            self.unmatched.pop_front();
        }
        self.unmatched.push_back(outcome);
    }

    /// Takes the FIRST queued outcome matching (sql, connection), leaving the rest.
    pub(crate) fn take_matching(
        &mut self,
        sql: &str,
        connection: &str,
    ) -> Option<AgentCaptureOutcome> {
        let position = self
            .unmatched
            .iter()
            .position(|o| o.matches(sql, connection))?;
        self.unmatched.remove(position)
    }

    /// Drops the queued outcomes (turn end, FIFO desync); the gap survives.
    pub(crate) fn clear_queue(&mut self) {
        self.unmatched.clear();
    }

    /// Whether no outcome is queued (the pairing tests assert this).
    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.unmatched.is_empty()
    }
}

/// Pairs a just-promoted pending query with its capture: the FIRST queued
/// outcome whose (sql, connection) equals the promoted query's — the
/// connection already filled at request time (S9). A captured outcome
/// becomes the capture slot with its agent evidence; a refused or absent
/// one clears the slot — never a silent fallback — and records the gap the
/// snapshot's refusal names. Returns a promoted capture's evidence, for the
/// transcript's provenance line.
pub(crate) fn promote_agent_capture(
    pending: &PendingQuery,
    captures: &mut AgentCaptures,
    captured: &mut Option<CapturedResult>,
) -> Option<ExecutionEvidence> {
    let connection = pending.connection.as_deref().unwrap_or("");
    match captures.take_matching(&pending.sql, connection) {
        Some(AgentCaptureOutcome::Captured(capture)) => {
            let evidence = agent_evidence(&capture);
            captures.gap = None;
            *captured = Some(CapturedResult {
                result: capture.result,
                evidence: evidence.clone(),
            });
            Some(evidence)
        }
        Some(AgentCaptureOutcome::Refused { reason, .. }) => {
            captures.gap = Some(match reason {
                CaptureRefusalReason::OverBudget => CaptureGap::OverBudget,
                CaptureRefusalReason::ModelViewTruncated => CaptureGap::ModelViewTruncated,
                CaptureRefusalReason::ModelViewRedacted => CaptureGap::ModelViewRedacted,
            });
            *captured = None;
            None
        }
        None => {
            captures.gap = Some(CaptureGap::Missing);
            *captured = None;
            None
        }
    }
}

#[cfg(test)]
#[path = "capture_agent_tests.rs"]
mod tests;
