//! The store's view of a session continuation: a typed version-1 record this
//! binary understands, or the verbatim JSON of one it does not.
//!
//! Preservation rules at this boundary:
//! - A continuation this binary does not understand — an unknown version, an
//!   unparsable shape, or a record over its bounds — loads as
//!   [`StoredContinuation::Unsupported`] and is written back verbatim. It is
//!   never dropped, reinterpreted, or half-updated, and its presence never
//!   fails the record's own deserialization.
//! - A save whose incoming record carries no continuation, or one this
//!   binary understands, may not overwrite an on-disk unsupported
//!   continuation: the store refuses with [`StoreError::UnsupportedContinuation`]
//!   before any disk write.
//! - Downgrade writes are unsupported: an older binary that does not know
//!   the field drops it on re-save. Restoring continuation state after a
//!   downgrade requires a pre-upgrade backup.

use crate::StoreError;
use saya_types::{ContinuationError, SessionContinuationV1};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// A session continuation as persisted: either the typed record this binary
/// reads and validates, or the raw JSON of one it does not. Both variants
/// serialize as the continuation's JSON object, so a newer binary reads what
/// an older binary preserved byte-for-byte.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredContinuation {
    Current(SessionContinuationV1),
    Unsupported(Value),
}

impl StoredContinuation {
    /// Strictly parses and validates `value`; any failure — unknown version,
    /// malformed shape, over-bound field — keeps it as
    /// [`Self::Unsupported`] so the session never fails to load.
    pub fn from_value(value: Value) -> Self {
        if let Ok(record) = serde_json::from_value::<SessionContinuationV1>(value.clone())
            && record.validate().is_ok()
        {
            Self::Current(record)
        } else {
            Self::Unsupported(value)
        }
    }

    /// Prepares a continuation for persistence. The typed record has its
    /// prose — narrative text, user notes, and check labels — redacted
    /// through the shared redactor, then revalidated, so secret-shaped
    /// material is refused or rewritten before any disk write. An
    /// unsupported record passes through untouched: it is not this binary's
    /// to interpret.
    pub(crate) fn prepare_for_persist(&mut self) -> Result<(), StoreError> {
        let Self::Current(record) = self else {
            return Ok(());
        };
        if let Some(narrative) = &mut record.narrative {
            narrative.text = crate::redact(&narrative.text);
        }
        if let Some(notes) = &mut record.user_notes {
            *notes = crate::redact(notes);
        }
        for check in &mut record.checks {
            check.label = crate::redact(&check.label);
        }
        record.validate().map_err(map_validation_error)
    }
}

/// Size failures are store limits; every other invalid shape is a plain
/// invalid value. Both are refusals before the save touches the disk.
fn map_validation_error(error: ContinuationError) -> StoreError {
    match error {
        ContinuationError::Oversize(_)
        | ContinuationError::NarrativeOversize(_)
        | ContinuationError::UserNotesOversize(_)
        | ContinuationError::FactPathOversize(_) => StoreError::LimitExceeded,
        _ => StoreError::Invalid,
    }
}

impl Serialize for StoredContinuation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Current(record) => record.serialize(serializer),
            Self::Unsupported(value) => value.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for StoredContinuation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::from_value(value))
    }
}
