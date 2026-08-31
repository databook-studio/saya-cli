//! Wiring the `[memory]` setting into the agent turn — spec E.
//!
//! Two pure translations live here, both provider-free so the behaviour the
//! spec governs — "the default configuration performs no automatic writes and
//! recalls nothing until enabled" — is unit-testable without a live model:
//!
//! - [`recall_mode_for`] / [`bounds_from`] map `mode` + the numeric bounds
//!   onto the [`RecallMode`](crate::contracts::RecallMode) the recall pipeline
//!   admits and the [`RecallBounds`] it enforces. `Off` becomes `None` so the
//!   caller skips recall entirely — not an empty block, no store query (§1).
//!   `Assisted` becomes `Some(RecallMode::IncludeCandidates)` to admit confirmed
//!   and candidate claims.
//! - [`LearningSetup::from`] maps `mode` onto candidate-write permission and the
//!   observation-log attachment. `Off` attaches **nothing**: not a disabled log,
//!   not an empty one — the collector does not exist, so the off path costs
//!   nothing (§2). `Assisted` attaches an observation log and permits candidate
//!   writes via `contract_propose`.

pub(crate) mod extractor;
pub(crate) mod extractor_prompt;
pub(crate) mod extractor_schema;
pub(crate) mod gate;
pub(crate) mod ingest;
pub(crate) mod profile_catalog;
pub(crate) mod resolver;
pub(crate) mod runner;
pub(crate) mod turn_record;
pub(crate) mod turn_table;

/// The wall-clock budget for one post-turn extraction call (spec packet-54
/// decision 5). The user already has their answer when extraction runs — it
/// trails the loop, after the assistant text — but the loop still awaits it, so
/// an adapter stays busy until it resolves. This bounds that wait, not the work
/// that produced the answer, and it is never unbounded: a half-open connection
/// would otherwise hold the session open indefinitely.
///
/// Set from measurement rather than estimate. Over 12 sequential turns against a
/// shared gateway, successful extractions ran 2.6s (min) / 4.7s (p50) / 13.1s
/// (max), and cost scales with how many objects the turn touched — single-object
/// turns finished near 4s and were never at risk, while every timeout was a
/// multi-object turn. At 15s a 13.1s extraction had under two seconds of
/// headroom on exactly the shape that was failing; 25s covers the observed worst
/// case with margin.
///
/// A fixed budget is admittedly the wrong shape for a cost that scales with
/// object count — a base plus per-object allowance would fit the curve better.
/// That is left undone deliberately: it is a tuning knob to maintain, and the
/// sample is sequential and single-gateway, so it does not exercise the
/// concurrent load the earlier 5s value was raised for. Widen the measurement
/// before adding the knob.
///
/// The runtime emits `KnowledgeLearningSkipped { TimedOut }` when it fires, and
/// `KnowledgeLearningStarted` when the call begins so the wait can be labelled.
pub(crate) const EXTRACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

#[allow(unused_imports)]
pub(crate) use extractor::parse_extraction_response;
#[allow(unused_imports)]
pub(crate) use extractor_prompt::build_extraction_prompt;
#[allow(unused_imports)]
pub(crate) use extractor_schema::{
    ExtractedProposal, ExtractionError, MAX_PROPOSALS_PER_EXTRACTION, ProposalOrigin,
};
#[allow(unused_imports)]
pub(crate) use gate::{GatingDecision, ProposalGating};
#[allow(unused_imports)]
pub(crate) use ingest::{
    IngestionError, filter_anti_self_reinforcement, filter_anti_self_reinforcement_dto,
    ingest_proposals,
};
#[allow(unused_imports)]
pub(crate) use resolver::{ResolutionError, ResolvedProposal, resolve_proposal, resolve_proposals};
#[allow(unused_imports)]
pub(crate) use runner::{ExtractionRunnerError, run_extraction};
#[allow(unused_imports)]
pub(crate) use turn_record::{
    MAX_ANSWER_BYTES, MAX_PROMPT_BYTES, MAX_TURN_RECORD_BYTES, SuppliedClaimDto,
    SuppliedContractDto, TurnRecord,
};
#[allow(unused_imports)]
pub(crate) use turn_table::{MAX_TURN_OBJECTS, TurnObjectEntry, TurnObjectId, TurnObjectTable};

use crate::agent::tools::ObservationLog;
use crate::contracts::{RecallBounds, RecallMode};
use saya_config::{MemoryMode, ResolvedMemory};
use std::sync::Arc;

/// The learning-derived inputs the runtime assembles a turn from.
///
/// `observations` is `None` for `mode = off`: the collector does not exist,
/// so recording is structurally impossible (not a guarded no-op). `Some` for
/// `assisted` — the log is attached so observations are recorded during the turn.
/// `permit_candidate_writes` is true only for `assisted`.
///
/// Not `Debug`/`Eq`: the observation log is a `Mutex` the runtime shares by
/// `Arc`, so neither derive is meaningful on this holder. The runtime reads the
/// two fields directly and clones the `Arc`.
pub(crate) struct LearningSetup {
    pub(crate) permit_candidate_writes: bool,
    pub(crate) observations: Option<Arc<ObservationLog>>,
}

impl LearningSetup {
    /// The `MemoryMode` → write permission + observation-log attachment.
    pub(crate) fn from(mode: MemoryMode) -> Self {
        match mode {
            MemoryMode::Off => Self {
                permit_candidate_writes: false,
                // Off attaches nothing: no collector is constructed, so the
                // record path is absent, not merely skipped.
                observations: None,
            },
            MemoryMode::Assisted => Self {
                permit_candidate_writes: true,
                observations: Some(Arc::new(ObservationLog::new())),
            },
            // `MemoryMode` is `#[non_exhaustive]`; a future variant the
            // runtime does not yet know about fails closed — no writes, no
            // collector — rather than guess a default that could store.
            _ => Self {
                permit_candidate_writes: false,
                observations: None,
            },
        }
    }

    /// True when this setup attaches an observation log.
    #[cfg(test)]
    pub(crate) fn observes(&self) -> bool {
        self.observations.is_some()
    }
}

/// The `MemoryMode` → the pipeline's admission mode, or `None` for `off`.
///
/// `None` means "do not recall": the caller produces no context block and does
/// not query the store (spec §1, `off`). `Some(IncludeCandidates)` is the
/// assisted mode: active claims are recalled, and pending candidates are admitted
/// to be labelled unconfirmed.
pub(crate) fn recall_mode_for(mode: MemoryMode) -> Option<RecallMode> {
    match mode {
        MemoryMode::Off => None,
        MemoryMode::Assisted => Some(RecallMode::IncludeCandidates),
        // `MemoryMode` is `#[non_exhaustive]`; a future variant the runtime
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

#[cfg(test)]
#[path = "../learning_tests.rs"]
mod tests;
