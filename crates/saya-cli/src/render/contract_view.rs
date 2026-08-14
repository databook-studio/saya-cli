//! Render-owned contract view DTOs.
//!
//! These are presentation types only. The `crate::contracts::view` types carry
//! data and IDs and must not gain `serde` or any presentation concern; the
//! adapter slice (2b-2c) maps one to the other.
//!
//! `profile` is the profile *name*, never the opaque identity. The identity is a
//! hash over host, database, user and scope path; serializing it would leak
//! material about the connection. There is no field for it here, and there must
//! not be one — that is a structural guarantee enforced by the type shape, not a
//! convention to remember (see `contract_view_serialized_keys_exclude_opaque_profile_identity`).

use serde::{Deserialize, Serialize};

/// One claim of a contract, in renderable form. `value` is a short rendered form
/// of the claim payload (e.g. a column name for `default_time_column`, an alias
/// for `table_alias`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractClaimView {
    pub claim_id: String,
    pub kind: String,
    pub origin: String,
    pub status: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
}

/// A disagreement between confirmed claims of an exclusive kind on one object.
/// Names the kind and the claim IDs only — never claim text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractConflictView {
    pub kind: String,
    pub claim_ids: Vec<String>,
}

/// One object's recallable contract, ready to render. `schema_state` is one of
/// `current`, `needs_review`, `stale`, `live_schema_unavailable`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractView {
    pub profile: String,
    pub object: String,
    pub schema_state: String,
    pub claims: Vec<ContractClaimView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<ContractConflictView>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// One candidate waiting for review, in renderable form. The queue is a flat
/// per-candidate listing — not the object-grouped `ContractView` shape — so it
/// gets its own DTO rather than a one-claim "contract" with a smuggled evidence
/// count. `profile` is the profile *name*, never the opaque identity, and
/// there is no field for the identity here either.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractQueueItemView {
    pub profile: String,
    pub claim_id: String,
    pub kind: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<String>,
    pub object: String,
    pub schema_state: String,
    pub evidence_count: usize,
}
