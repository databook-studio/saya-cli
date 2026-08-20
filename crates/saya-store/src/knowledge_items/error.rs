//! Typed refusals for the [`crate::knowledge_items`] repository.
//!
//! The repository's job is to refuse the three ways a caller can get
//! cardinality wrong: a payload that does not match the slot it is filed
//! under, a multi-valued slot pushed past its bound, and a slot string that
//! is not a slot at all. Each is a distinct, payload-free error so a caller
//! can branch on what went wrong without parsing a message. Infrastructure
//! failures (the store is down, a row would collide with the storage-boundary
//! invariant) come through the [`StoreError`](crate::StoreError) variants they
//! already share with the rest of the store.

use crate::StoreError;
use thiserror::Error;

/// A typed refusal from the knowledge-items repository.
///
/// Payload-free by the security standard: a variant names what went wrong,
/// never the value, slot, or object the caller tried to act on.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum KnowledgeStoreError {
    /// The payload's kind does not match the slot it was filed under — for
    /// example, a `ClaimPayload::TableAlias` under `KnowledgeSlot::TableGrain`.
    /// Reusing `ClaimPayload`'s serialisation is cheap, but a `relationship`
    /// payload has no slot at all, so the binding must agree or nothing is
    /// written. The slot and payload are kept out of the message.
    #[error("the payload does not match the knowledge slot")]
    CardinalityMismatch,
    /// A multi-valued slot was pushed past its declared bound. The bound is
    /// declared on the slot in `saya-types`, so the repository reads it from
    /// the slot rather than re-deriving a rule.
    #[error("the slot already holds its maximum number of values")]
    BoundExceeded,
    /// A slot string that is not a valid `KnowledgeSlot`, or a row in the table
    /// whose stored `slot`/`cardinality`/`value_json` do not reconstruct. The
    /// former is a caller error; the latter means the store was written by a
    /// build this one cannot read, so the read fails closed rather than
    /// guessing.
    #[error("the knowledge slot is not valid")]
    MalformedSlot,
    /// The local state store is unavailable, or a write collided with the
    /// storage-boundary unique index — which is itself the contract a
    /// single-valued slot admits one row, so a collision the repository did
    /// not resolve as a replace surfaces here as a conflict.
    #[error(transparent)]
    Store(#[from] StoreError),
}
