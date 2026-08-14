//! Wiring the `[memory]` settings into the agent turn — spec Phase 4b.
//!
//! Two translations live here, both pure and provider-free so the behaviour the
//! spec governs — "the default configuration performs no automatic writes and
//! recalls exactly what it recalled before" — is unit-testable without a live
//! model:
//!
//! - [`recall_mode_for`] / [`bounds_from`] map `recall` + the numeric bounds
//!   onto the [`RecallMode`](crate::contracts::RecallMode) the recall pipeline
//!   admits and the [`RecallBounds`] it enforces. `Off` becomes `None` so the
//!   caller skips recall entirely — not an empty block, no store query (§1).
//! - [`LearningSetup::from`] maps `learning` onto the candidate-write
//!   permission and the observation-log attachment. `Off` attaches **nothing**:
//!   not a disabled log, not an empty one — the collector does not exist, so
//!   the off path costs nothing (§2). The `suggest` post-turn report lives in
//!   [`report`].
//!
//! The five-second extraction bound (§3) is **not** honoured by spawning work
//! — see [`report::suggest_report`]'s docs for the decision.

mod report;

pub(crate) use report::suggest_report;

use crate::agent::tools::ObservationLog;
use crate::contracts::{RecallBounds, RecallMode};
use saya_agent::{AgentEvent, AgentEventSink};
use saya_config::{MemoryLearning, MemoryRecall, ResolvedMemory};
use std::sync::Arc;

/// The learning-derived inputs the runtime assembles a turn from.
///
/// `observations` is `None` for `learning = off`: the collector does not exist,
/// so recording is structurally impossible (not a guarded no-op). `Some` for
/// `suggest` and `auto-candidate` — the log is attached so the turn can report
/// or persist what it observed. `permit_candidate_writes` is true only for
/// `auto-candidate`; `suggest` observes but never writes (spec §2 table).
///
/// Not `Debug`/`Eq`: the observation log is a `Mutex` the runtime shares by
/// `Arc`, so neither derive is meaningful on this holder. The runtime reads the
/// two fields directly and clones the `Arc`.
pub(crate) struct LearningSetup {
    pub(crate) permit_candidate_writes: bool,
    pub(crate) observations: Option<Arc<ObservationLog>>,
}

impl LearningSetup {
    /// The `learning` mode → write permission + observation-log attachment.
    pub(crate) fn from(learning: MemoryLearning) -> Self {
        match learning {
            MemoryLearning::Off => Self {
                permit_candidate_writes: false,
                // Off attaches nothing: no collector is constructed, so the
                // record path is absent, not merely skipped.
                observations: None,
            },
            MemoryLearning::Suggest => Self {
                permit_candidate_writes: false,
                observations: Some(Arc::new(ObservationLog::new())),
            },
            MemoryLearning::AutoCandidate => Self {
                permit_candidate_writes: true,
                observations: Some(Arc::new(ObservationLog::new())),
            },
            // `MemoryLearning` is `#[non_exhaustive]`; a future variant the
            // runtime does not yet know about fails closed — no writes, no
            // collector — rather than guess a default that could store.
            _ => Self {
                permit_candidate_writes: false,
                observations: None,
            },
        }
    }

    /// True when this setup attaches an observation log the runtime drains after
    /// the turn — `suggest` reports from it; `auto-candidate` persisted during.
    #[cfg(test)]
    pub(crate) fn observes(&self) -> bool {
        self.observations.is_some()
    }
}

/// The `recall` mode → the pipeline's admission mode, or `None` for `off`.
///
/// `None` means "do not recall": the caller produces no context block and does
/// not query the store (spec §1, `off`). `Some(Confirmed)` is today's behaviour;
/// `Some(IncludeCandidates)` widens the filter so candidates reach the render
/// layer to be labelled unconfirmed.
pub(crate) fn recall_mode_for(recall: MemoryRecall) -> Option<RecallMode> {
    match recall {
        MemoryRecall::Off => None,
        MemoryRecall::Confirmed => Some(RecallMode::Confirmed),
        MemoryRecall::IncludeCandidates => Some(RecallMode::IncludeCandidates),
        // `MemoryRecall` is `#[non_exhaustive]`; a future variant the runtime
        // does not yet know about fails closed — no recall — rather than guess.
        _ => None,
    }
}

/// The numeric bounds from `[memory]` → the pipeline's `RecallBounds`. The
/// config layer already range-validates these, so this is a straight copy.
pub(crate) fn bounds_from(memory: &ResolvedMemory) -> RecallBounds {
    RecallBounds {
        max_objects: memory.max_contracts as usize,
        max_claims_per_object: memory.max_claims_per_contract as usize,
        max_bytes: memory.max_context_bytes as usize,
    }
}

/// Emits the `suggest`-mode report after a turn, if the turn was `suggest`,
/// completed (`ok`), and proposal-worthy. Nothing is stored — `contract_propose`
/// was hidden, so the model never wrote a claim; the report is the evidence
/// base (spec 4b §2, test 7). Only a completed turn reports: a cancelled or
/// errored turn is not "per completed turn", and reporting its partial
/// observations would mislead. Emitted as an assistant-text line so it reaches
/// both the terminal and the TUI channel without a new event variant.
pub(crate) async fn emit_suggest_report(
    learning: MemoryLearning,
    log: Option<&Arc<ObservationLog>>,
    ok: bool,
    sink: &dyn AgentEventSink,
) {
    if learning != MemoryLearning::Suggest || !ok {
        return;
    }
    let Some(log) = log else { return };
    let drained = log.drain();
    if let Some(report) = suggest_report(&drained) {
        sink.emit(AgentEvent::assistant_text(report)).await;
    }
}

#[cfg(test)]
#[path = "../learning_tests.rs"]
mod tests;
