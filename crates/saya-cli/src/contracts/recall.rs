//! Recall: selecting and ranking contracts for an agent's context block.

use crate::contracts::assemble::assemble;
use crate::contracts::selection::select;
use crate::contracts::view::{RecallDiagnostics, RecallOutcome};
use saya_store::SqliteStateStore;
use saya_types::{DatabaseObjectRef, ProfileIdentity, SchemaTree};

/// Bounds for a recall, from plan §11.2.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RecallBounds {
    pub max_objects: usize,
    pub max_claims_per_object: usize,
    pub max_bytes: usize,
}

impl Default for RecallBounds {
    fn default() -> Self {
        Self::defaults()
    }
}

impl RecallBounds {
    pub(crate) const fn defaults() -> Self {
        Self {
            max_objects: 5,
            max_claims_per_object: 12,
            max_bytes: 16384,
        }
    }
}

/// Which claim statuses a recall admits. `Off` never reaches here — the caller
/// skips recall entirely when there is nothing to recall — so this enum models
/// only the two modes the pipeline distinguishes. Kept in the contracts layer
/// (not `saya_config::MemoryRecall`) so the typed operations stay free of the
/// config crate; the agent runtime maps the config enum onto this.
///
/// `Confirmed` is today's behaviour: only confirmed claims are recallable.
/// `IncludeCandidates` admits `Candidate` claims too, so the render layer can
/// show them plainly labelled as unconfirmed (ADR 0002 §4: inference is not
/// confirmation; including a candidate is the user opting to see it anyway).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecallMode {
    #[default]
    Confirmed,
    IncludeCandidates,
}

impl RecallMode {
    /// Whether `status` is admitted by this mode. `Confirmed` keeps today's
    /// `is_recallable` filter; `IncludeCandidates` widens it to candidates.
    pub(crate) fn admits(self, status: saya_types::ClaimStatus) -> bool {
        use saya_types::ClaimStatus;
        match self {
            Self::Confirmed => status.is_recallable(),
            // A candidate joins confirmed claims; every other non-confirmed
            // status (rejected, stale, contradicted, forgotten) stays excluded.
            Self::IncludeCandidates => {
                matches!(status, ClaimStatus::Confirmed | ClaimStatus::Candidate)
            }
        }
    }
}

/// A recall request. `schemas` carries the live schema per active profile so
/// validity can compare the stored fingerprint to the live one without this
/// module re-deriving it; the caller already holds live schema for query-building.
pub(crate) struct RecallRequest<'a> {
    pub profiles: &'a [ProfileIdentity],
    pub explicit_refs: &'a [DatabaseObjectRef],
    pub terms: &'a [String],
    pub allow_database_context: bool,
    pub schemas: &'a [(ProfileIdentity, SchemaTree)],
    pub bounds: RecallBounds,
    /// Which statuses this recall admits. Defaults to `Confirmed` (today's
    /// behaviour); `IncludeCandidates` widens the filter so candidates reach
    /// the render layer to be shown labelled as unconfirmed.
    pub recall_mode: RecallMode,
}

/// Runs a recall against `store`. Store failure returns an empty outcome with
/// `store_unavailable: true` — recall degrades answer quality, never the query path.
pub(crate) async fn recall(store: &SqliteStateStore, request: RecallRequest<'_>) -> RecallOutcome {
    let selection = match select(store, &request, request.schemas).await {
        Ok(s) => s,
        Err(_) => {
            return empty(RecallDiagnostics {
                attempted: true,
                store_unavailable: true,
                ..default_diag()
            });
        }
    };

    let mut diag = RecallDiagnostics {
        attempted: true,
        considered: selection.considered,
        excluded_by_status: selection.excluded_by_status,
        ..default_diag()
    };

    // Privacy gate: evaluate after selection so the count of suppressed objects
    // is meaningful, but before assembling any contract.
    if !request.allow_database_context {
        diag.excluded_by_privacy = selection.candidates.len();
        return empty(diag);
    }

    let contracts = assemble(
        &selection.candidates,
        request.schemas,
        request.bounds,
        &mut diag,
    );
    diag.selected = contracts.len();
    RecallOutcome {
        contracts,
        diagnostics: diag,
    }
}

fn default_diag() -> RecallDiagnostics {
    RecallDiagnostics::default()
}

fn empty(diag: RecallDiagnostics) -> RecallOutcome {
    RecallOutcome {
        contracts: Vec::new(),
        diagnostics: diag,
    }
}
