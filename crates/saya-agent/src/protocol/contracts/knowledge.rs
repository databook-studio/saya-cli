//! Knowledge/recall DTOs: claims and contracts supplied to a turn, claims
//! proposed by it, and findings when the turn's SQL contradicted a confirmed
//! claim — the payload that crosses the boundary inside `AgentEvent`.

use saya_types::{ClaimId, ClaimStatus};
use serde::{Deserialize, Serialize};

/// The three distinguishable states of a turn's recall, as
/// [`AgentEvent::KnowledgeSupplied`] carries them. `Off` (recall
/// disabled by config), `Skipped` (the privacy gate closed — SAYA was not
/// allowed to look), and `Ran` (recall ran against the store) are three facts a
/// user reads differently; collapsing them into a single "no event" would hide
/// the distinction between "SAYA was not allowed to look" and "SAYA looked and
/// had nothing". `Ran { store_unavailable: true }` records a store failure that
/// degraded recall to an empty result — the turn still completes (recall is
/// fail-soft, spec §3).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KnowledgeOutcome {
    /// Recall is off by config; SAYA did not look.
    Off,
    /// The privacy gate closed; SAYA was not allowed to look. No store query.
    Skipped,
    /// Recall ran against the store. `store_unavailable` is true when a store
    /// failure degraded recall to an empty result.
    Ran { store_unavailable: bool },
}

/// One claim as **supplied** to a turn's context block, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeSupplied`]. Carries
/// the claim id (so a later phase can name exactly which saved claims shaped
/// an answer), its kind, the short rendered value the prompt block shows, a
/// column when the claim is column-scoped, and its persisted status — so a
/// `Candidate` reads as `candidate`, distinct from `confirmed`.
///
/// No raw payload, evidence, or SQL. `value` is the same short rendered form
/// the prompt block already shows (a column name, an alias), not the stored
/// payload — and it is named **supplied**, never *used*: a confirmed claim
/// being supplied does not mean the generated SQL honoured it (we have
/// measured that it frequently does not).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuppliedClaimDto {
    pub claim_id: ClaimId,
    /// The claim kind token (`table_alias`, `default_time_column`, …).
    pub kind: String,
    /// The short rendered value the prompt block shows, not the stored payload.
    pub value: String,
    /// A column name when the claim is column-scoped; `None` for table-level
    /// claims. `skip_serializing_if` keeps it off the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub status: ClaimStatus,
}

/// One object's claims, as supplied to the turn, in the DTO shape that crosses
/// the crate boundary into [`AgentEvent::KnowledgeSupplied`]. `profile` is the
/// human-facing profile **name**, never the opaque [`saya_types::ProfileIdentity`]
/// — the identity has no field here, by construction. `schema_state`
/// is the contract's aggregated state token (`current` / `needs_review` /
/// `live_schema_unavailable`); `stale` never appears (a contract aggregating to
/// `Stale` is dropped by the model-path policy before supply).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuppliedContractDto {
    /// The human-facing profile name. Never the opaque identity.
    pub profile: String,
    /// The object's qualified name (`catalog.schema.object`).
    pub object: String,
    /// The aggregated schema state token; `stale` never appears here.
    pub schema_state: String,
    pub claims: Vec<SuppliedClaimDto>,
}

/// One candidate claim **proposed** (persisted) this turn, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeProposed`].
/// Mirrors [`SuppliedClaimDto`]'s vocabulary — same `claim_id` / `kind` / `value`
/// / `column` / `status` — and adds `profile` and `object`, because a proposal is
/// a single flat claim, not a claim nested under a contract stanza. `profile` is
/// the human-facing profile **name**, never the opaque
/// [`saya_types::ProfileIdentity`] (no identity field, by construction).
///
/// `value` is the same short rendered form a later recall would show (a column
/// name, an alias), reusing the recall render path's `claim_value` so a proposal
/// can never name a value recall would not — not the stored payload. `status` is
/// the status the claim *landed with*: `contract_propose` stores only a
/// `Candidate`, so a `KnowledgeProposed` event never reads as established (a
/// candidate is inert until a human confirms it). No raw SQL, evidence, or cells.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedClaimDto {
    pub claim_id: ClaimId,
    /// The human-facing profile name. Never the opaque identity.
    pub profile: String,
    /// The object's qualified name (`catalog.schema.object`).
    pub object: String,
    /// The claim kind token (`table_alias`, `default_time_column`, …).
    pub kind: String,
    /// The short rendered value, not the stored payload.
    pub value: String,
    /// A column name when the claim is column-scoped; `None` for table-level
    /// claims. `skip_serializing_if` keeps it off the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    /// The status the claim landed with — `Candidate` for a proposal.
    pub status: ClaimStatus,
}

/// One confirmed claim the turn's SQL contradicted, in the DTO shape that
/// crosses the crate boundary into [`AgentEvent::KnowledgeOverridden`] (spec
/// A1). Mirrors nothing about a claim being *used* — the finding says the claim
/// was contradicted and names the time-named columns the SQL **referenced**
/// instead, which is all the extractor can prove from names. `claimed_value`
/// is the value the claim specifies (the claimed time column), carried so a
/// render can say "where you specified Y". No opaque identity, no raw SQL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OverrideFindingDto {
    pub claim_id: ClaimId,
    /// The claim kind token. Only `default_time_column` is ever produced.
    pub kind: String,
    /// The value the claim specifies — for `default_time_column`, the claimed
    /// time column. "Where you specified Y" in the render.
    pub claimed_value: String,
    /// Time-named columns the SQL referenced instead, as written, sorted for
    /// determinism. Observed references, not an asserted "used" column.
    pub observed_columns: Vec<String>,
}
