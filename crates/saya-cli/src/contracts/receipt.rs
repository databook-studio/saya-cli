//! A typed receipt for what a recall **supplied** to the model.
//!
//! This is a contract, not presentation: it carries data and IDs only, mirrors
//! the doc style of [`super::view`], and lives here (not in a render module) for
//! the same reason [`RecallOutcome`] does — the adapter slice that renders it is
//! a later slice. Nothing displays it yet; P1b consumes it. See
//! `.claude/specs/spec-p1a-recall-receipt.md`.
//!
//! ## What this names, and what it deliberately does not
//!
//! The receipt names what recall **supplied** to the prompt — the claims whose
//! rendered lines reached the context block — never what the model **used**.
//! Recall injects context; the model may ignore it. There is no field here that
//! could be read as "the model applied this claim", because none was measured.
//! `supplied` / `included` are the load-bearing words; `applied` / `used` are
//! avoided on purpose (spec §3, a correctness property).
//!
//! `profile` is the human-facing profile **name**, never the opaque
//! [`ProfileIdentity`]. The identity is a hash over connection material; the
//! receipt has no field for it, by construction — see the structural test
//! `receipt_carries_no_profile_identity_field`.
//!
//! No raw SQL, evidence text, or result cells appear here. `value` is the same
//! short rendered form the prompt block already shows (a column name, an alias),
//! not the claim's stored payload.

use saya_types::{ClaimId, ClaimStatus};

/// The receipt for one recall: what it supplied, what bounds dropped, and
/// whether recall ran at all. Returned beside the context blocks by
/// [`crate::agent::recall_context::recall_context_blocks`].
///
/// `dropped_by_bounds` counts **claims** dropped by the byte or count caps
/// after recall's policy filter — distinct from claims a schema-staleness policy
/// dropped (those never reach the supply path and are surfaced separately via
/// [`super::view::RecallDiagnostics`]). A non-zero `dropped_by_bounds` is the
/// receipt's way of saying "the list above is a subset, not the whole"; zero
/// means the supply path kept everything it selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecallReceipt {
    /// Whether recall ran, or was skipped by policy or configuration. See [`RecallOutcomeKind`].
    pub kind: RecallOutcomeKind,
    /// One entry per object whose claims reached the rendered context block,
    /// in the order they appear in the block. Empty when recall did not run
    /// or when a recall matched nothing.
    pub supplied: Vec<SuppliedContract>,
    /// Claims dropped by the byte or count caps (not by schema policy). Zero
    /// when nothing was selected or recall did not run.
    pub dropped_by_bounds: usize,
}

impl RecallReceipt {
    /// A receipt for a recall that matched nothing and dropped nothing.
    pub(crate) fn ran_empty(store_unavailable: bool) -> Self {
        Self {
            kind: RecallOutcomeKind::Ran { store_unavailable },
            supplied: Vec::new(),
            dropped_by_bounds: 0,
        }
    }

    /// A receipt for a recall disabled by configuration (`recall = off`).
    /// SAYA was configured not to look; no store query was performed.
    pub(crate) fn configured_off() -> Self {
        Self {
            kind: RecallOutcomeKind::ConfiguredOff,
            supplied: Vec::new(),
            dropped_by_bounds: 0,
        }
    }

    /// A receipt for a recall skipped because the privacy gate is closed.
    /// SAYA was not permitted to query the store or supply database contracts.
    pub(crate) fn privacy_gate_closed() -> Self {
        Self {
            kind: RecallOutcomeKind::PrivacyGateClosed,
            supplied: Vec::new(),
            dropped_by_bounds: 0,
        }
    }
}

/// Whether a recall ran against the store, or was not run due to configuration
/// or policy.
///
/// The distinction the receipt exists to make: an empty `supplied` under
/// [`RecallOutcomeKind::Ran`] means "recall ran and matched nothing" (or the
/// store was unavailable); under [`RecallOutcomeKind::ConfiguredOff`] it means
/// "recall was configured off"; under [`RecallOutcomeKind::PrivacyGateClosed`]
/// it means "recall was skipped because the privacy gate was closed" — three
/// distinct facts, never collapsible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecallOutcomeKind {
    /// Recall ran (or attempted) against the store. `store_unavailable` is
    /// true when a store failure degraded the recall to an empty result; the
    /// turn still completes (recall is fail-soft). When false and `supplied`
    /// is empty, recall ran and matched nothing.
    Ran { store_unavailable: bool },
    /// Recall was disabled by configuration (`recall = off`); SAYA did not look.
    /// No store query was performed, and no claims were selected or dropped.
    ConfiguredOff,
    /// Recall was skipped because the privacy gate is closed (`allow_query_data`
    /// is false); SAYA was not permitted to look. No store query was performed,
    /// and no contract content reaches a provider.
    PrivacyGateClosed,
}

/// One object's claims, as supplied to the prompt. `schema_state` is the
/// contract's aggregated state (see [`super::view::ContractSchemaState`]) — a
/// per-object fact, stated once here rather than repeated on every claim.
/// `profile` is the profile **name**, never the opaque identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuppliedContract {
    /// The human-facing profile name. Never the opaque identity.
    pub profile: String,
    /// The object's qualified name (`catalog.schema.object`).
    pub object: String,
    /// The aggregated schema state token (`current` / `needs_review` /
    /// `live_schema_unavailable`). `stale` never appears here: a contract that
    /// aggregates to `Stale` is dropped by the model-path policy before supply.
    pub schema_state: &'static str,
    /// The claims of this object that reached the rendered block.
    pub claims: Vec<SuppliedClaim>,
}

/// One claim as supplied to the prompt. Carries the claim id (so a later phase
/// can name exactly which saved claims shaped an answer), its kind, a short
/// rendered value, and its persisted status — so a `Candidate` reads as
/// `candidate`, not flattened into a single "included" notion (P1b renders it
/// differently; P3 acts on it). No raw payload, evidence, or SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuppliedClaim {
    pub claim_id: ClaimId,
    /// The claim kind token (`table_alias`, `default_time_column`, …).
    pub kind: &'static str,
    /// The short rendered value the prompt block shows (a column name, an
    /// alias, a description). Not the stored payload.
    pub value: String,
    /// A column name when the claim is column-scoped; `None` for table-level
    /// claims.
    pub column: Option<String>,
    /// The claim's persisted status word (`confirmed` or `candidate` on the
    /// recall path; every other status is filtered out before supply).
    pub status: ClaimStatus,
}
