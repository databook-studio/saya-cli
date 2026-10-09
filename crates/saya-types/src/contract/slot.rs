//! Typed knowledge slots with declared cardinality.
//!
//! A [`KnowledgeSlot`] names *what* a piece of knowledge is about — a position
//! on an object, scoped to a column where the position is column-level — and
//! carries the cardinality rule for that position on the slot itself. This is
//! the vocabulary the later adopting slice uses to replace the conflict
//! subsystem: a single-valued slot has one current value, so a correction
//! *replaces* it instead of producing a second confirmed claim that a separate
//! pass must later detect as a disagreement.
//!
//! Nothing here is wired to storage yet. The slot names a position; the value
//! remains a `ClaimPayload`, unchanged. Cardinality is enforced by the
//! [`SlotValues`] holder (in [`slot_values`](crate::contract::slot_values)),
//! which is the one place an append is refused for a single-valued slot —
//! there is no method that can silently grow a single-valued slot to two
//! values.

use serde::{Deserialize, Serialize};

use crate::contract::identity::validate_name;

/// Bound on every multi-valued slot. Multi-valued slots render into the
/// recall context block, so an unbounded list is a context-budget problem.
/// Four keeps the worst case modest: four `table.description` values at
/// `MAX_TEXT_CHARS` (1024) each is ~4 KB, a small slice of the conversation
/// byte budget; aliases are short names and negligible at four. Four is large
/// enough that a table is never forced to drop a legitimate second alias or
/// clarification, and small enough that a runaway list cannot crowd out the
/// question the context is meant to help.
pub const MAX_MULTI_SLOT_VALUES: usize = 4;

/// The position a piece of knowledge occupies on a database object.
///
/// Column-scoped variants carry the column name, so
/// `column:created_at.role` and `column:updated_at.role` are *different*
/// single-valued slots. The string form ([`as_str`](Self::as_str)) is the
/// stable identifier that is compared, stored and rendered; it round-trips
/// through [`parse`](Self::parse). Serialisation uses that same string form,
/// so a stored slot is its canonical identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub enum KnowledgeSlot {
    TableDescription,
    TableAlias,
    TableGrain,
    TableDefaultTime,
    ColumnDescription {
        column: String,
    },
    ColumnRole {
        column: String,
    },
    /// A join condition the user taught, scoped to the local table the claim
    /// is stored against. The joined table is carried in the payload, so the
    /// slot itself is table-level: one table can join many targets, and several
    /// rules for the same target coexist as alternatives.
    RelationJoinRule,
    /// A business metric defined over the table the claim is stored against.
    /// The metric's underlying columns are carried in the payload so the
    /// binding can invalidate the fact when one of them disappears.
    MetricDefinition,
    /// A user-authored note bound to this table, preserved verbatim.
    TableUserNote,
}

impl KnowledgeSlot {
    /// The canonical string form. Returns an owned `String` (not `&'static
    /// str` like the fieldless contract enums) because the column-scoped
    /// variants carry a column name; the sibling carrying type
    /// `DatabaseObjectRef::qualified_name` returns `String` for the same
    /// reason.
    pub fn as_str(&self) -> String {
        match self {
            Self::TableDescription => "table.description".to_owned(),
            Self::TableAlias => "table.alias".to_owned(),
            Self::TableGrain => "table.grain".to_owned(),
            Self::TableDefaultTime => "table.default_time".to_owned(),
            Self::ColumnDescription { column } => {
                format!("column:{column}.description")
            }
            Self::ColumnRole { column } => format!("column:{column}.role"),
            Self::RelationJoinRule => "relation.join_rule".to_owned(),
            Self::MetricDefinition => "metric.definition".to_owned(),
            Self::TableUserNote => "table.user_note".to_owned(),
        }
    }

    /// Inverse of [`as_str`](Self::as_str). Returns `None` for an unknown or
    /// malformed slot rather than panicking. A column name that fails
    /// `validate_name` (empty, too long, control characters) is not a slot,
    /// so it too yields `None`. Parsing strips a fixed suffix (`.description`
    /// / `.role`), not a split on `.`, so a column name containing dots is
    /// unambiguous.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "table.description" => Some(Self::TableDescription),
            "table.alias" => Some(Self::TableAlias),
            "table.grain" => Some(Self::TableGrain),
            "table.default_time" => Some(Self::TableDefaultTime),
            "relation.join_rule" => Some(Self::RelationJoinRule),
            "metric.definition" => Some(Self::MetricDefinition),
            "table.user_note" => Some(Self::TableUserNote),
            _ => {
                let rest = value.strip_prefix("column:")?;
                if let Some(col) = rest.strip_suffix(".description") {
                    validate_name(col).ok()?;
                    Some(Self::ColumnDescription {
                        column: col.to_owned(),
                    })
                } else if let Some(col) = rest.strip_suffix(".role") {
                    validate_name(col).ok()?;
                    Some(Self::ColumnRole {
                        column: col.to_owned(),
                    })
                } else {
                    None
                }
            }
        }
    }

    /// How many values this slot admits, declared on the slot so any holder
    /// can ask without consulting a table elsewhere. Single-valued slots
    /// admit one; multi-valued slots admit [`MAX_MULTI_SLOT_VALUES`].
    pub const fn cardinality(&self) -> SlotCardinality {
        match self {
            Self::TableDescription | Self::TableAlias | Self::ColumnDescription { .. } => {
                SlotCardinality::Multi {
                    max: MAX_MULTI_SLOT_VALUES,
                }
            }
            Self::TableGrain | Self::TableDefaultTime | Self::ColumnRole { .. } => {
                SlotCardinality::Single
            }
            // A table joins many targets, and a table carries many metrics;
            // distinct values are distinct rows, refused past the bound.
            Self::RelationJoinRule | Self::MetricDefinition | Self::TableUserNote => {
                SlotCardinality::Multi {
                    max: MAX_MULTI_SLOT_VALUES,
                }
            }
        }
    }

    /// The column this slot is scoped to, or `None` for a table-level slot.
    pub fn column(&self) -> Option<&str> {
        match self {
            Self::ColumnDescription { column } | Self::ColumnRole { column } => Some(column),
            _ => None,
        }
    }
}

impl std::fmt::Display for KnowledgeSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_str())
    }
}

impl From<KnowledgeSlot> for String {
    fn from(slot: KnowledgeSlot) -> String {
        slot.as_str()
    }
}

impl TryFrom<String> for KnowledgeSlot {
    type Error = SlotParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or(SlotParseError(()))
    }
}

/// Error surfaced only through `Deserialize` / `TryFrom<String>` when a
/// string is not a valid slot. Payload-free and message-only: it never
/// echoes the rejected string. Not part of the append/replace enforcement
/// surface — that uses [`SlotError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotParseError(());

impl std::fmt::Display for SlotParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a valid knowledge slot")
    }
}

impl std::error::Error for SlotParseError {}

/// How many values a [`KnowledgeSlot`] admits, reported by the slot itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotCardinality {
    /// Exactly one value; a correction replaces it.
    Single,
    /// Up to `max` values; appends are refused past `max`.
    Multi { max: usize },
}

impl SlotCardinality {
    /// The number of values the slot admits: `1` for single, `max` for multi.
    pub const fn max(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Multi { max } => max,
        }
    }

    /// True for a single-valued slot.
    pub const fn is_single(self) -> bool {
        matches!(self, Self::Single)
    }
}

#[cfg(test)]
mod tests;
