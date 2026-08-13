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
