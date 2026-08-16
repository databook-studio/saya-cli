//! The error every shared contract operation returns.
//!
//! Split out of `review.rs` so the operations read as a list of operations.
//! Each variant names a refusal a caller must be able to act on — a silent
//! no-op would let a caller believe it changed something it did not.

use saya_store::StoreError;
use thiserror::Error;

/// variant added later does not silently become an unhandled case in an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub(crate) enum ContractOpError {
    #[error("the requested claim does not exist")]
    NotFound,
    #[error("the operation conflicts with an existing claim")]
    Conflict,
    #[error("the value is not valid for storage")]
    Invalid,
    #[error("the value exceeds a store limit")]
    Limit,
    #[error("the state store is unavailable")]
    Unavailable,
    /// Revalidating a Stale claim needs a live schema to fingerprint against,
    /// and none was available — no cached schema for the claim's profile, or
    /// the store could not be read. Distinct from [`Self::Unavailable`]: the
    /// store may be fine; the schema cache is what is missing.
    #[error("no schema is available to revalidate the claim against")]
    SchemaUnavailable,
    /// The claim's table is not in the live schema, so there is nothing to
    /// revalidate against. Distinct from [`Self::Conflict`]: nothing conflicts
    /// — the object is simply gone.
    #[error(
        "the table this claim describes is no longer in the schema; forget the claim, or refresh if the table still exists"
    )]
    ObjectGone,
    /// A column the claim depends on is absent from the live table. Confirming
    /// would revive a claim whose dependency vanished, so the user must point
    /// it somewhere real or drop it.
    #[error(
        "a column this claim depends on is gone; edit the claim to name a column that exists, or forget it"
    )]
    ColumnGone,
    /// [`use_candidate_once`] was called on a claim that is not a live candidate.
    /// Confirmed is refused too — already admissible by the mode, so a silent
    /// success would let a caller believe it did something it did not (spec C §5.5).
    #[error("the claim is not a live candidate; only an unconfirmed candidate may be used once")]
    NotACandidate,
}

impl From<StoreError> for ContractOpError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::NotFound => Self::NotFound,
            StoreError::Conflict => Self::Conflict,
            StoreError::Invalid => Self::Invalid,
            StoreError::LimitExceeded => Self::Limit,
            StoreError::Unavailable | StoreError::VersionUnsupported => Self::Unavailable,
            // StoreError is #[non_exhaustive]; a future variant is a store
            // problem the adapter cannot route around, so it degrades to
            // Unavailable rather than becoming an unhandled case.
            _ => Self::Unavailable,
        }
    }
}
