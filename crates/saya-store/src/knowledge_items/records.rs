//! Request and record types for the knowledge-items repository.
//!
//! Object identity is inlined on the row — the point of D-3 — so a
//! [`KnowledgeItemRequest`] carries a full [`DatabaseObjectRef`] rather than a
//! foreign key to `contract_objects`. A read hands back the same identity on a
//! [`KnowledgeItem`], so a caller never joins to learn "what does SAYA know
//! about this object".
//!
//! `value_json` reuses [`ClaimPayload`]'s serialisation: each knowledge slot
//! maps one-to-one onto a `ClaimPayload` variant, so one validation path
//! (the `claim_payload` constructors that reject control characters, length,
//! and bad names) keeps oversized and structured text out of the rendered
//! context block. A `relationship` payload has no slot; the repository refuses
//! that mismatch as a typed error rather than storing a payload the slots
//! vocabulary cannot name.

use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectRef, KnowledgeSlot, KnowledgeState, SchemaFingerprint,
};

/// Bound on the serialised `value_json`. Matches the existing claim payload
/// bound: a knowledge item is the same kind of bounded business statement, so
/// the same ceiling keeps a runaway payload out of the context block.
pub const MAX_KNOWLEDGE_ITEM_BYTES: usize = 4096;

/// A single current-state knowledge row, as read back from the table. Object
/// identity is inlined, not joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeItem {
    /// Stable id derived from the object identity and slot, so a re-write of
    /// the same single-valued slot lands on the same row.
    pub id: String,
    pub object: DatabaseObjectRef,
    pub slot: KnowledgeSlot,
    pub cardinality_single: bool,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    pub state: KnowledgeState,
    /// The schema binding the row was written under, serialised as JSON. Opaque
    /// to the store — it is the caller's record of what the object looked like,
    /// stored beside the `fingerprint_version` that says which format it was
    /// computed under.
    pub schema_binding_json: String,
    pub fingerprint_version: u32,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

/// A write to the knowledge-items table. The repository decides replace vs.
/// insert from the slot's cardinality: a single-valued slot replaces the one
/// row the unique index permits; a multi-valued slot appends, refused past its
/// bound. The caller does not pick.
#[derive(Debug, Clone)]
pub struct KnowledgeItemRequest {
    pub object: DatabaseObjectRef,
    pub slot: KnowledgeSlot,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    pub state: KnowledgeState,
    /// Pre-serialised schema binding, opaque to the store. The caller — which
    /// has the live or cached schema — builds this; the store never interprets
    /// it, only persists it beside the fingerprint version.
    pub schema_binding_json: String,
    pub fingerprint: SchemaFingerprint,
}
