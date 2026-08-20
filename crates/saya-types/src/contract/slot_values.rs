//! The cardinality-enforcing holder for a [`KnowledgeSlot`].
//!
//! [`SlotValues`] is the one place cardinality is enforced: `append` on a
//! single-valued slot is a typed error even when the slot is empty, so there
//! is no operation that can silently grow a single-valued slot to two values;
//! a multi-valued slot refuses past its declared bound. The slot names the
//! position and declares the rule; the value type `T` is whatever the caller
//! brings (the adopting slice will bind it to a payload), so this holder does
//! not duplicate `ClaimPayload`.

use crate::contract::slot::KnowledgeSlot;

/// A typed error from the [`SlotValues`] append/replace operations. Payload-free
/// by the security standard: it names what went wrong, never the value or slot
/// the caller tried to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SlotError {
    /// `append` was called on a single-valued slot. Single-valued slots take
    /// `replace`; there is no append path that can grow one to two values.
    #[error("slot is single-valued; use replace, not append")]
    AppendToSingle,
    /// `replace` was called on a multi-valued slot. Multi-valued slots take
    /// `append`; `clear` then re-append to reset.
    #[error("slot is multi-valued; use append, not replace")]
    ReplaceOnMulti,
    /// `append` would exceed the slot's declared bound.
    #[error("slot already holds its maximum number of values")]
    CapacityExceeded,
}

/// A [`KnowledgeSlot`] paired with the values currently held against it. The
/// slot's declared [`Cardinality`](crate::SlotCardinality) is the rule this
/// holder enforces: `append` works only on multi-valued slots and refuses past
/// the bound; `replace` works only on single-valued slots and always leaves
/// exactly one value. The wrong operation for the slot's cardinality is a
/// typed error, so a single-valued slot cannot be grown to two values by any
/// method on this type.
#[derive(Debug, Clone)]
pub struct SlotValues<T> {
    slot: KnowledgeSlot,
    values: Vec<T>,
}

impl<T> SlotValues<T> {
    /// An empty holder for `slot`.
    pub fn new(slot: KnowledgeSlot) -> Self {
        Self {
            slot,
            values: Vec::new(),
        }
    }

    /// The slot these values are held against.
    pub fn slot(&self) -> &KnowledgeSlot {
        &self.slot
    }

    /// The current values, in insertion order.
    pub fn values(&self) -> &[T] {
        &self.values
    }

    /// How many values are currently held.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when no values are held.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Drop every held value. Cardinality-agnostic; the slot keeps its rule.
    pub fn clear(&mut self) {
        self.values.clear();
    }

    /// Append a value to a multi-valued slot. Refused as a typed error when the
    /// slot is single-valued (use [`replace`](Self::replace)) or when the slot
    /// is already at its declared bound.
    pub fn append(&mut self, value: T) -> Result<(), SlotError> {
        let cardinality = self.slot.cardinality();
        if cardinality.is_single() {
            return Err(SlotError::AppendToSingle);
        }
        if self.values.len() >= cardinality.max() {
            return Err(SlotError::CapacityExceeded);
        }
        self.values.push(value);
        Ok(())
    }

    /// Replace the single value of a single-valued slot, returning the value
    /// that was there before. Refused as a typed error on a multi-valued slot.
    /// Always leaves exactly one value, never two.
    pub fn replace(&mut self, value: T) -> Result<Option<T>, SlotError> {
        if !self.slot.cardinality().is_single() {
            return Err(SlotError::ReplaceOnMulti);
        }
        match self.values.first_mut() {
            Some(current) => Ok(Some(std::mem::replace(current, value))),
            None => {
                self.values.push(value);
                Ok(None)
            }
        }
    }
}
