//! Stable id derivation for knowledge rows.
//!
//! A knowledge row's id is derived from the object identity and the slot, so a
//! re-write of the same single-valued slot lands on the same row the unique
//! index already keys on. The same length-prefixed field hashing the contract
//! keys use, so two profiles with the same qualified name and slot do not
//! alias (the profile id is part of the hash).

use saya_types::{DatabaseObjectRef, KnowledgeSlot};
use sha2::{Digest, Sha256};

fn field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value.as_bytes());
}

/// `ki-` + 64 hex digits, derived from the object identity and the slot's
/// canonical string form. Used for single-valued slots, where one row per
/// object+slot means the value must not take part in the id — a re-write lands
/// on the same row.
pub(crate) fn knowledge_item_id(object: &DatabaseObjectRef, slot: &KnowledgeSlot) -> String {
    let mut hash = Sha256::new();
    field(&mut hash, object.profile().as_str());
    field(&mut hash, object.catalog());
    field(&mut hash, object.schema());
    field(&mut hash, object.object());
    field(&mut hash, object.kind().as_str());
    field(&mut hash, &slot.as_str());
    hex_id(&mut hash)
}

/// The `ki-…` id a [`put_knowledge_item`](super::KnowledgeItemStore::put_knowledge_item)
/// write of `value` under `slot` on `object` would land on — the row a re-write
/// or a re-file of the same value resolves to. Single-valued slots key on
/// `(object, slot)`; multi-valued slots key on `(object, slot, value)`.
///
/// `serialized_value` must be the same `serde_json::to_string(&value)` the write
/// path uses, so the id this returns is the id the row was stored under. Exposed
/// so a caller can ask "is there already a row for this fact?" by id — the
/// question `remember`'s dedup asks — without the store interpreting payload
/// semantics, and without a decoded-value comparison that a forgotten (blanked)
/// tombstone would fail. The tombstone keeps its id, so an id lookup still
/// finds it where a value comparison would miss.
pub fn knowledge_item_id_for(
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    serialized_value: &str,
) -> String {
    if slot.cardinality().is_single() {
        knowledge_item_id(object, slot)
    } else {
        knowledge_item_id_value(object, slot, serialized_value)
    }
}

/// A multi-valued slot's id takes the serialised value into account, so two
/// distinct values for the same object+slot are two rows, and re-filing the
/// same value lands on the same row (an idempotent update, not a duplicate).
pub(crate) fn knowledge_item_id_value(
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
    serialized_value: &str,
) -> String {
    let mut hash = Sha256::new();
    field(&mut hash, object.profile().as_str());
    field(&mut hash, object.catalog());
    field(&mut hash, object.schema());
    field(&mut hash, object.object());
    field(&mut hash, object.kind().as_str());
    field(&mut hash, &slot.as_str());
    field(&mut hash, serialized_value);
    hex_id(&mut hash)
}

fn hex_id(hash: &mut Sha256) -> String {
    let digest = hash.finalize_reset();
    let mut value = String::from("ki-");
    for byte in digest {
        value.push_str(&format!("{byte:02x}"));
    }
    value
}
