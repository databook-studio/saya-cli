//! Spec D-2 — one computed validity over the new state vocabulary.
//!
//! `KnowledgeValidity` is the verdict a knowledge item earns against the schema
//! known for its profile. The legacy `StoredClaim`-based classifier
//! (`schema_state_for` over `contract_claims`) and its `ClaimStatus`→`KnowledgeState`
//! mapping were deleted in Chunk 5 with the legacy store they read; the
//! knowledge path is the only path, and [`item_validity_for`] is the verdict it
//! uses. [`KnowledgeValidity`] survives because the render/retrieval layers
//! still speak [`crate::contracts::view::ContractSchemaState`], and the
//! `From<KnowledgeValidity> for ContractSchemaState` impl is the bridge.
//!
//! Why a separate enum at all: `ContractSchemaState::Stale` is *computed*, but
//! the legacy `ClaimStatus::Stale` was *persisted* — the same fact had two homes
//! and they could disagree. D-1 introduced [`KnowledgeState`] with no `Stale`
//! variant because staleness is a read-time computation, not a stored state.
//! [`item_validity_for`] owns that computation: `Invalid` is what a gone bound
//! column reads as, and it is never persisted.
//!
//! [`Dismissed`]: saya_types::KnowledgeState::Dismissed

use crate::contracts::availability::{SchemaAvailability, SchemaFreshness};
use crate::contracts::view::ContractSchemaState;
use saya_store::KnowledgeItem;
use saya_types::{FINGERPRINT_VERSION, KnowledgeState, SchemaBinding};

/// The computed validity of a claim against the schema known for its profile.
///
/// Ordered worst-first for aggregation via [`Self::aggregate`]:
/// `Invalid` beats `SchemaUnavailable` beats `NeedsReview` beats `Valid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnowledgeValidity {
    /// The claim matches the schema it was made against (← `Current`).
    Valid,
    /// The schema moved in a way a human must judge (← `NeedsReview`). Not
    /// dropped on the model path: one unrelated column added to a wide table
    /// must not mute every claim on it.
    NeedsReview,
    /// No usable schema: the cache is missing, unreadable, or (on the model
    /// path) past the freshness bound (← `LiveSchemaUnavailable`). Never
    /// `Invalid` — an absent cache is not evidence a column is gone.
    SchemaUnavailable,
    /// A referenced column the claim depends on is gone, or changed in a way
    /// that can silently break it (← `Stale`). A contract that aggregates to
    /// `Invalid` is not safe for the model to read as a fact.
    Invalid,
}

impl KnowledgeValidity {
    /// A claim cannot be more reassuring than the worst claim in the same
    /// contract — one `Invalid` claim must flag the object, not hide behind a
    /// `Valid` sibling. Mirrors [`ContractSchemaState::aggregate`].
    pub(crate) fn aggregate(self, other: Self) -> Self {
        let rank = |state: Self| match state {
            Self::Invalid => 3,
            Self::SchemaUnavailable => 2,
            Self::NeedsReview => 1,
            Self::Valid => 0,
        };
        if rank(self) >= rank(other) {
            self
        } else {
            other
        }
    }
}

/// Bridge to the render/retrieval vocabulary. [`assemble`] and [`show`] compute
/// a [`KnowledgeValidity`] per item and fold it into the contract's
/// [`ContractSchemaState`] via this `into()`, so the render and receipt layers
/// keep speaking the four-state vocabulary they were built on.
///
/// [`assemble`]: super::assemble
/// [`show`]: super::show
impl From<KnowledgeValidity> for ContractSchemaState {
    fn from(validity: KnowledgeValidity) -> Self {
        match validity {
            KnowledgeValidity::Valid => Self::Current,
            KnowledgeValidity::NeedsReview => Self::NeedsReview,
            KnowledgeValidity::SchemaUnavailable => Self::LiveSchemaUnavailable,
            KnowledgeValidity::Invalid => Self::Stale,
        }
    }
}

/// The computed validity of a [`KnowledgeItem`] under the D-4 binding model.
///
/// The item carries its structural dependency as a serialised [`SchemaBinding`]
/// (`Table` | `Column { column, requirement }`) plus a `fingerprint_version`.
/// D-4's contract is that a fact depends only on what it names, so validity is
/// [`SchemaBinding::validate_in_tree`] against the live schema — an unrelated
/// column changing never invalidates (the false-alarm "needs review" the old
/// whole-table fingerprint model produced is gone by design; see
/// [`SchemaBinding`]). The four-state [`KnowledgeValidity`] is produced by the
/// gates that wrap the binding check:
///
/// - [`KnowledgeState::Dismissed`] → [`Invalid`](KnowledgeValidity::Invalid)
///   without a schema lookup. A dismissed item is withdrawn from use; no
///   revalidation revives it, and it must never read `Valid`. This fires
///   before the schema lookup so a missing or unreadable cache cannot upgrade
///   it to `SchemaUnavailable` (which is kept on the model path).
/// - No live schema (`Missing`, `Unavailable`, or — on the model path — a
///   cached schema older than the freshness bound) →
///   [`SchemaUnavailable`](KnowledgeValidity::SchemaUnavailable). An absent or
///   unreadable cache is not evidence a bound column is gone.
/// - A `fingerprint_version` this build did not write →
///   [`NeedsReview`](KnowledgeValidity::NeedsReview). The binding was derived
///   under a format this build cannot faithfully compare, so the item is held
///   for human review rather than trusted or silently dropped.
/// - A binding JSON this build cannot deserialize (a row written by an
///   incompatible build) → `NeedsReview`, for the same reason.
/// - Otherwise the deserialised binding is validated against the live tree:
///   `Valid` if every dependency it names is satisfied, `Invalid` if a bound
///   column is gone or lost its required semantic type.
///
/// Pure: no store, no clock, no I/O. `freshness` is an input so the model path
/// can pass a controlled stamp and the human path can pass `Unbounded`.
pub(crate) fn item_validity_for(
    item: &KnowledgeItem,
    availability: &SchemaAvailability,
    freshness: SchemaFreshness,
) -> KnowledgeValidity {
    if matches!(item.state, KnowledgeState::Dismissed) {
        return KnowledgeValidity::Invalid;
    }
    let Some(schema) = availability.live_table_schema(freshness) else {
        return KnowledgeValidity::SchemaUnavailable;
    };
    if item.fingerprint_version != FINGERPRINT_VERSION {
        return KnowledgeValidity::NeedsReview;
    }
    match serde_json::from_str::<SchemaBinding>(&item.schema_binding_json) {
        Ok(binding) => {
            let obj = &item.object;
            match binding.validate_in_tree(schema, obj.catalog(), obj.schema(), obj.object()) {
                saya_types::BindingValidity::Valid => KnowledgeValidity::Valid,
                saya_types::BindingValidity::Invalid => KnowledgeValidity::Invalid,
            }
        }
        // A binding this build cannot interpret was written by an incompatible
        // build. Hold the item for human review rather than dropping a
        // possibly-valid fact or trusting an uninterpretable one.
        Err(_) => KnowledgeValidity::NeedsReview,
    }
}

#[cfg(test)]
#[path = "knowledge_validity/tests.rs"]
mod tests;
