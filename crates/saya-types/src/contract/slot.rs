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
            Self::RelationJoinRule | Self::MetricDefinition => SlotCardinality::Multi {
                max: MAX_MULTI_SLOT_VALUES,
            },
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
mod tests {
    use super::*;
    use crate::contract::slot_state::KnowledgeState;
    use crate::contract::slot_values::{SlotError, SlotValues};

    /// Every slot's string form round-trips (spec §5.1). Column-scoped slots
    /// are exercised with a column name that contains a dot, to prove the
    /// suffix-strip parse is unambiguous.
    #[test]
    fn slot_string_round_trips() {
        let slots = [
            KnowledgeSlot::TableDescription,
            KnowledgeSlot::TableAlias,
            KnowledgeSlot::TableGrain,
            KnowledgeSlot::TableDefaultTime,
            KnowledgeSlot::ColumnDescription {
                column: "created_at".into(),
            },
            KnowledgeSlot::ColumnDescription {
                column: "x.description".into(),
            },
            KnowledgeSlot::ColumnRole {
                column: "updated_at".into(),
            },
            KnowledgeSlot::ColumnRole {
                column: "x.role".into(),
            },
            KnowledgeSlot::RelationJoinRule,
            KnowledgeSlot::MetricDefinition,
        ];
        for slot in slots {
            let s = slot.as_str();
            assert_eq!(KnowledgeSlot::parse(&s), Some(slot.clone()), "{s}");
        }
    }

    /// A single-valued slot reports cardinality 1; a multi-valued one reports
    /// its bound (spec §5.2).
    #[test]
    fn cardinality_max_matches_kind() {
        for single in [
            KnowledgeSlot::TableGrain,
            KnowledgeSlot::TableDefaultTime,
            KnowledgeSlot::ColumnRole { column: "c".into() },
        ] {
            assert!(single.cardinality().is_single());
            assert_eq!(single.cardinality().max(), 1);
        }
        for multi in [
            KnowledgeSlot::TableDescription,
            KnowledgeSlot::TableAlias,
            KnowledgeSlot::ColumnDescription { column: "c".into() },
            KnowledgeSlot::RelationJoinRule,
            KnowledgeSlot::MetricDefinition,
        ] {
            assert!(!multi.cardinality().is_single());
            assert_eq!(multi.cardinality().max(), MAX_MULTI_SLOT_VALUES);
        }
    }

    /// The relation and metric slots are table-level: they carry no column, and a
    /// table admits several of each (a table joins many targets and carries
    /// many metrics), so they are multi-valued rather than single.
    #[test]
    fn relation_and_metric_slots_are_table_scoped_and_multi() {
        for slot in [
            KnowledgeSlot::RelationJoinRule,
            KnowledgeSlot::MetricDefinition,
        ] {
            assert_eq!(slot.column(), None, "{slot} is table-scoped");
            assert!(!slot.cardinality().is_single(), "{slot} is multi-valued");
            assert_eq!(slot.cardinality().max(), MAX_MULTI_SLOT_VALUES);
        }
        // The wire spellings round-trip and stay distinct from every other slot.
        assert_eq!(
            KnowledgeSlot::RelationJoinRule.as_str(),
            "relation.join_rule"
        );
        assert_eq!(
            KnowledgeSlot::MetricDefinition.as_str(),
            "metric.definition"
        );
    }

    /// Two column-scoped slots for different columns are different slots
    /// (spec §5.3).
    #[test]
    fn column_scoped_slots_distinct_by_column() {
        let a = KnowledgeSlot::parse("column:created_at.role").unwrap();
        let b = KnowledgeSlot::parse("column:updated_at.role").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.column(), Some("created_at"));
        assert_eq!(b.column(), Some("updated_at"));
        // Same column is the same slot.
        assert_eq!(
            KnowledgeSlot::parse("column:created_at.role"),
            KnowledgeSlot::parse("column:created_at.role")
        );
    }

    /// Replacing the value of a single-valued slot yields one value, not two
    /// (spec §5.4).
    #[test]
    fn replace_on_single_yields_one_value() {
        let mut slot = SlotValues::new(KnowledgeSlot::TableGrain);
        assert!(slot.is_empty());
        assert!(slot.replace("first").unwrap().is_none());
        assert_eq!(slot.len(), 1);
        assert_eq!(slot.values(), &["first"]);

        let old = slot.replace("second").unwrap();
        assert_eq!(old, Some("first"));
        assert_eq!(slot.len(), 1, "replace must not grow a single-valued slot");
        assert_eq!(slot.values(), &["second"]);
    }

    /// Exceeding a multi-valued bound is refused as a typed error (spec §5.5).
    #[test]
    fn append_past_bound_is_a_typed_error() {
        let mut slot = SlotValues::new(KnowledgeSlot::TableAlias);
        for i in 0..MAX_MULTI_SLOT_VALUES {
            assert!(slot.append(format!("a{i}")).is_ok());
        }
        assert_eq!(slot.len(), MAX_MULTI_SLOT_VALUES);
        assert_eq!(
            slot.append("one too many".to_string()).unwrap_err(),
            SlotError::CapacityExceeded
        );
        // The refused append left the slot unchanged.
        assert_eq!(slot.len(), MAX_MULTI_SLOT_VALUES);
    }

    /// An unknown slot string parses to `None` rather than panicking (spec
    /// §5.6). Covers an unknown table kind, an unknown column kind, a
    /// missing column, and an invalid column name.
    #[test]
    fn unknown_slot_parses_to_none() {
        assert_eq!(KnowledgeSlot::parse("table.bogus"), None);
        assert_eq!(KnowledgeSlot::parse("column:x.bogus"), None);
        assert_eq!(KnowledgeSlot::parse("column:.role"), None, "empty column");
        assert_eq!(KnowledgeSlot::parse("column.role"), None, "no column");
        assert_eq!(KnowledgeSlot::parse("table.description.extra"), None);
        assert_eq!(KnowledgeSlot::parse(""), None);
        assert_eq!(KnowledgeSlot::parse("not_a_slot"), None);
        // A known slot family with an unknown member is still rejected, not
        // guessed: a misspelled relation or metric kind yields None.
        assert_eq!(KnowledgeSlot::parse("relation.bogus"), None);
        assert_eq!(KnowledgeSlot::parse("metric.bogus"), None);
        assert_eq!(KnowledgeSlot::parse("relation.join_rule.extra"), None);
    }

    /// Hard constraint (the one the task says it checks hardest): appending a
    /// second value to a single-valued slot must fail as a typed error — and
    /// here, even appending the *first* value via `append` is refused, because
    /// a single-valued slot only takes `replace`. There is no operation in
    /// this API that can grow a single-valued slot to two values.
    #[test]
    fn append_to_single_is_a_typed_error_even_when_empty() {
        let mut slot = SlotValues::new(KnowledgeSlot::ColumnRole {
            column: "created_at".into(),
        });
        assert!(slot.is_empty());
        assert_eq!(slot.append("first").unwrap_err(), SlotError::AppendToSingle);
        assert!(slot.is_empty(), "a refused append must store nothing");

        // The single-valued path still works and stays single.
        slot.replace("only").unwrap();
        assert_eq!(slot.len(), 1);
        assert_eq!(
            slot.append("second").unwrap_err(),
            SlotError::AppendToSingle
        );
        assert_eq!(slot.len(), 1);
    }

    /// `replace` on a multi-valued slot is a typed error: the multi path is
    /// `append`, not `replace`. Symmetric with the constraint above.
    #[test]
    fn replace_on_multi_is_a_typed_error() {
        let mut slot = SlotValues::new(KnowledgeSlot::TableDescription);
        assert_eq!(slot.replace("x").unwrap_err(), SlotError::ReplaceOnMulti);
        assert!(slot.is_empty(), "a refused replace must store nothing");
    }

    /// `KnowledgeState` round-trips through its string form and serde, and
    /// rejects unknown strings.
    #[test]
    fn knowledge_state_round_trips() {
        for state in [
            KnowledgeState::Pending,
            KnowledgeState::Active,
            KnowledgeState::Dismissed,
        ] {
            assert_eq!(KnowledgeState::parse(state.as_str()), Some(state));
        }
        assert_eq!(KnowledgeState::parse("unknown"), None);

        // serde form equals the as_str form (snake_case), so a stored state
        // is its canonical identifier.
        let json = serde_json::to_string(&KnowledgeState::Active).unwrap();
        assert_eq!(json, r#""active""#);
        let back: KnowledgeState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, KnowledgeState::Active);
    }

    /// A slot serialises to its canonical string form and deserialises back,
    /// so a stored slot is its stable identifier (round-trips with `parse`).
    #[test]
    fn slot_serde_uses_string_form() {
        let slot = KnowledgeSlot::ColumnRole {
            column: "created_at".into(),
        };
        let json = serde_json::to_string(&slot).unwrap();
        assert_eq!(json, r#""column:created_at.role""#);
        let back: KnowledgeSlot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, slot);
        // A malformed slot string fails deserialisation, it is not silently
        // coerced.
        let bad: Result<KnowledgeSlot, _> = serde_json::from_str(r#""table.bogus""#);
        assert!(bad.is_err());
    }
}
