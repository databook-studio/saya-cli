//! Recall: selecting and ranking contracts for an agent's context block.

use crate::contracts::assemble::assemble;
use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::retrieval::{self, RetrievalPolicy};
use crate::contracts::selection::select;
use crate::contracts::view::{RecallDiagnostics, RecallOutcome};
use saya_store::SqliteStateStore;
use saya_types::{DatabaseObjectRef, ProfileIdentity};

/// Bounds for a recall, from plan §11.2.
///
/// `max_objects` and `max_claims_per_object` are count bounds [`recall`]/`assemble`
/// apply to the contracts they return. `max_bytes` is **not** applied by `recall`:
/// it bounds the *rendered* block the prompt-recall caller sends, and what reaches
/// the request is the rendered body (headers, markers, conflict lines), not the
/// serialized payloads `assemble` sees. Measuring payloads here would bound the
/// wrong unit, so the byte bound lives in `crate::agent::recall_context`, against
/// the rendered body and the agent message budget. Callers that do not render a
/// prompt block (the `contract_search` tool) pass it through unused.
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

/// Which knowledge states a recall admits. `Off` never reaches here — the
/// caller skips recall entirely when there is nothing to recall — so this
/// enum models only the two modes the pipeline distinguishes. Kept in the
/// contracts layer (not `saya_config::MemoryRecall`) so the typed operations
/// stay free of the config crate; the agent runtime maps the config enum
/// onto this.
///
/// `Confirmed` admits only `Active` items (the D-3 state for a confirmed
/// fact). `IncludeCandidates` admits `Pending` items too, so the render
/// layer can show them plainly labelled as unconfirmed (ADR 0002 §4:
/// inference is not confirmation; including a candidate is the user opting
/// to see it anyway). `Dismissed` is never admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum RecallMode {
    #[default]
    Confirmed,
    IncludeCandidates,
}

impl RecallMode {
    /// Whether a persisted [`KnowledgeState`] is admitted by this mode.
    ///
    /// `Confirmed` admits only `Active` (a binding fact); `IncludeCandidates`
    /// widens to `Pending` so the render layer can show an unconfirmed
    /// inference plainly labelled. `Dismissed` is never admitted — it is the
    /// withdrawn state (rejected/forgotten/contradicted) and recall never
    /// supplies it. This is the D-3 translation of the old
    /// `admits(ClaimStatus)` filter: `Active ↔ Confirmed`, `Pending ↔
    /// Candidate`, `Dismissed ↔ every non-recallable status`.
    pub(crate) fn admits_state(self, state: saya_types::KnowledgeState) -> bool {
        use saya_types::KnowledgeState;
        match self {
            Self::Confirmed => matches!(state, KnowledgeState::Active),
            Self::IncludeCandidates => {
                matches!(state, KnowledgeState::Active | KnowledgeState::Pending)
            }
        }
    }
}

/// A recall request. `schemas` carries the schema known per active profile so
/// validity can compare the stored fingerprint to the known one without this
/// module re-deriving it. Each entry is a [`SchemaAvailability`] — `Missing` or
/// `Unavailable` classifies `LiveSchemaUnavailable`, never `Stale`, so a store
/// hiccup or undiscovered profile cannot mute a claim as drift.
///
/// `now_unix_ms` is the instant the model path bounds cached-schema freshness
/// against: a schema older than [`super::availability::MODEL_SCHEMA_MAX_AGE_MS`]
/// cannot classify a claim as `Current`. The human-review path (`ForHumanReview`)
/// ignores it and uses the cache regardless of age.
pub(crate) struct RecallRequest<'a> {
    pub profiles: &'a [ProfileIdentity],
    pub explicit_refs: &'a [DatabaseObjectRef],
    pub terms: &'a [String],
    pub allow_database_context: bool,
    pub schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    pub now_unix_ms: i64,
    pub bounds: RecallBounds,
    /// Which statuses this recall admits. Defaults to `Confirmed` (today's
    /// behaviour); `IncludeCandidates` widens the filter so candidates reach
    /// the render layer to be shown labelled as unconfirmed.
    pub recall_mode: RecallMode,
    /// One candidate [`use_candidate_once`](super::use_once::use_candidate_once)
    /// admitted to *this* recall despite `recall_mode`. `None` is today's
    /// behaviour: the mode alone decides. `Some(id)` lets exactly that one
    /// `Pending` item through selection under `Confirmed`, without promoting
    /// it — the item keeps its state, so the render layer still marks it
    /// `[candidate — unconfirmed]`.
    ///
    /// Request-scoped by construction: the field lives on the request, which
    /// is built and consumed once per recall and then dropped, so an admission
    /// cannot survive the turn it was made for. Selection honours the exception
    /// only for a live `Pending`
    /// item — a non-pending id here is a no-op, because
    /// [`use_candidate_once`] refuses to mint an admission for anything but a
    /// live candidate, so a dismissed or active id never reaches a request.
    pub admit_candidate: Option<saya_types::ClaimId>,
    /// Who the result is for. `ForModel` (the default) drops a contract whose
    /// computed schema state is `Stale` and counts it in `excluded_by_schema`;
    /// `ForHumanReview` keeps stale contracts so a human can act on them. The
    /// default is `ForModel` because `recall` feeds prompt context and the
    /// agent's `contract_search` — the human-facing `contracts list` opts into
    /// `ForHumanReview`. See [`retrieval`] and [`RetrievalPolicy`].
    pub policy: RetrievalPolicy,
}

/// Runs a recall against `store`. Store failure returns an empty outcome with
/// `store_unavailable: true` — recall degrades answer quality, never the query path.
///
/// Reads the D-3 `knowledge_items` table; a store error from that read
/// (`KnowledgeStoreError`) degrades to an empty `Ran { store_unavailable }`
/// outcome, the same fail-soft the legacy `contract_claims` read had.
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

    let assembled = assemble(
        &selection.candidates,
        request.schemas,
        request.bounds,
        freshness_for(request.policy, request.now_unix_ms),
        &mut diag,
    );
    // One policy, applied here for every recall caller: a contract computed
    // `Stale` is dropped for `ForModel` (and counted), kept for `ForHumanReview`.
    let contracts = retrieval::apply(assembled, request.policy, &mut diag);
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

/// The freshness a recall applies to each profile's cached schema. The model
/// path bounds age against `now_unix_ms`: a stale-by-age cache cannot vouch
/// for currency and so classifies `LiveSchemaUnavailable`. The human-review
/// path is unbounded — `contracts list`/`show`/`queue` use the cache to show
/// what it knows, not to trust a query built on it.
fn freshness_for(policy: RetrievalPolicy, now_unix_ms: i64) -> SchemaFreshness {
    match policy {
        RetrievalPolicy::ForModel => SchemaFreshness::for_model(now_unix_ms),
        RetrievalPolicy::ForHumanReview => SchemaFreshness::Unbounded,
    }
}
