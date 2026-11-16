use serde::{Deserialize, Serialize};

use crate::contract::claim_enums::ColumnRole;
use crate::contract::claim_payload::ClaimPayload;
use crate::contract::slot::KnowledgeSlot;
use crate::contract::type_classifier::{is_numeric_type, is_temporal_type};
use crate::schema::{SchemaTree, Table};

/// Verdict of validating a knowledge item's schema binding against live table metadata.
///
/// This is strictly a binary verdict: either all schema dependencies required
/// by the business fact exist and satisfy semantic type expectations (`Valid`),
/// or a required dependency is missing or contradicted (`Invalid`).
///
/// Prior whole-table fingerprint systems used a three-way verdict with `NeedsReview`
/// whenever unreferenced columns changed, which caused false alarms ("crying wolf")
/// and eroded trust in persistent knowledge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingValidity {
    Valid,
    Invalid,
}

impl BindingValidity {
    /// True when the binding satisfies all live schema dependencies.
    pub const fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }

    /// True when any live schema dependency is missing or contradicted.
    pub const fn is_invalid(self) -> bool {
        matches!(self, Self::Invalid)
    }
}

/// Semantic type requirement placed on a depended-upon column.
///
/// Rather than snapshotting exact connector type strings (which breaks on
/// harmless type widenings like `VARCHAR(20)` -> `VARCHAR(50)` or `INT` -> `BIGINT`),
/// bindings record the semantic capability the fact requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColumnRequirement {
    /// The column must exist in the live table, with any data type.
    #[default]
    Exists,
    /// The column must exist and possess a temporal type (date, time, timestamp).
    Time,
    /// The column must exist and possess a numeric type (int, float, decimal, etc.).
    Numeric,
}

/// Structural dependencies a piece of knowledge requires from the database schema.
///
/// A fact depends only on what it actually uses:
/// - Table-level knowledge (`TableDescription`, `TableAlias`, `TableGrain`) depends
///   only on the table existing.
/// - Column-scoped knowledge (`ColumnDescription`, `ColumnRole`, `TableDefaultTime`)
///   depends on the specific named column existing and optionally satisfying a
///   semantic type constraint ([`ColumnRequirement`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SchemaBinding {
    /// Depends only on the table existing in the schema.
    Table,
    /// Depends on a named column existing and satisfying `requirement`.
    Column {
        column: String,
        #[serde(default)]
        requirement: ColumnRequirement,
    },
    /// Depends on each named column existing, with any data type. Used by a
    /// fact that rests on several columns at once — a join rule's local keys,
    /// a metric's underlying columns — so the fact is invalid the moment any
    /// one of them disappears, and undisturbed by an unrelated column changing.
    /// A semantic type requirement is not recorded here: a join key or a
    /// metric's column set is too varied for a single structural type check,
    /// and disappearance is the failure that matters.
    Columns { columns: Vec<String> },
}

impl SchemaBinding {
    /// Derives the structural schema binding for a paired `(KnowledgeSlot, ClaimPayload)`.
    ///
    /// Table-level slots (`TableDescription`, `TableAlias`, `TableGrain`) yield
    /// `SchemaBinding::Table`.
    /// Column-level slots yield `SchemaBinding::Column` with appropriate semantic
    /// `ColumnRequirement`:
    /// - `TableDefaultTime` and `ColumnRole::Timestamp` require `ColumnRequirement::Time`.
    /// - `ColumnRole::Measure` requires `ColumnRequirement::Numeric`.
    /// - `ColumnDescription` and other `ColumnRole` variants require `ColumnRequirement::Exists`.
    ///
    /// Returns `None` if the slot and payload types disagree, if column names mismatch,
    /// or if the payload is not slot-bound (e.g. `ClaimPayload::Relationship`).
    pub fn derive(slot: &KnowledgeSlot, payload: &ClaimPayload) -> Option<Self> {
        match (slot, payload) {
            (KnowledgeSlot::TableUserNote, ClaimPayload::TableUserNote { .. })
            | (KnowledgeSlot::TableDescription, ClaimPayload::TableDescription { .. })
            | (KnowledgeSlot::TableAlias, ClaimPayload::TableAlias { .. })
            | (KnowledgeSlot::TableGrain, ClaimPayload::TableGrain { .. }) => Some(Self::Table),
            (KnowledgeSlot::TableDefaultTime, ClaimPayload::DefaultTimeColumn { column, .. }) => {
                Some(Self::Column {
                    column: column.clone(),
                    requirement: ColumnRequirement::Time,
                })
            }
            (
                KnowledgeSlot::ColumnDescription { column: slot_col },
                ClaimPayload::ColumnDescription { column, .. },
            ) if slot_col == column => Some(Self::Column {
                column: column.clone(),
                requirement: ColumnRequirement::Exists,
            }),
            (
                KnowledgeSlot::ColumnRole { column: slot_col },
                ClaimPayload::ColumnRole { column, role, .. },
            ) if slot_col == column => {
                let requirement = match role {
                    ColumnRole::Timestamp => ColumnRequirement::Time,
                    ColumnRole::Measure => ColumnRequirement::Numeric,
                    ColumnRole::Identifier | ColumnRole::Dimension | ColumnRole::Sensitive => {
                        ColumnRequirement::Exists
                    }
                };
                Some(Self::Column {
                    column: column.clone(),
                    requirement,
                })
            }
            // A join rule depends on its local join keys — the columns on the
            // table the claim is stored against. A rule with no keys (a
            // predicate-only join no declared constraint describes) can name no
            // column to depend on, so it falls back to the table existing.
            (KnowledgeSlot::RelationJoinRule, ClaimPayload::JoinRule { local_columns, .. }) => {
                if local_columns.is_empty() {
                    Some(Self::Table)
                } else {
                    Some(Self::Columns {
                        columns: local_columns.clone(),
                    })
                }
            }
            // A metric depends on the columns its definition is built from; a
            // metric with no column references binds to the table existing.
            (KnowledgeSlot::MetricDefinition, ClaimPayload::MetricDefinition { columns, .. }) => {
                if columns.is_empty() {
                    Some(Self::Table)
                } else {
                    Some(Self::Columns {
                        columns: columns.clone(),
                    })
                }
            }
            _ => None,
        }
    }

    /// Validates this binding against a live table definition.
    ///
    /// Evaluates whether the referenced table and column dependencies are
    /// satisfied. Matching column names is case-insensitive to align with SQL
    /// catalog conventions across database dialects.
    pub fn validate(&self, table: &Table) -> BindingValidity {
        match self {
            Self::Table => BindingValidity::Valid,
            Self::Column {
                column,
                requirement,
            } => {
                let Some(col) = table
                    .columns
                    .iter()
                    .find(|c| c.name.eq_ignore_ascii_case(column))
                else {
                    return BindingValidity::Invalid;
                };

                match requirement {
                    ColumnRequirement::Exists => BindingValidity::Valid,
                    ColumnRequirement::Time => {
                        if is_temporal_type(&col.data_type) {
                            BindingValidity::Valid
                        } else {
                            BindingValidity::Invalid
                        }
                    }
                    ColumnRequirement::Numeric => {
                        if is_numeric_type(&col.data_type) {
                            BindingValidity::Valid
                        } else {
                            BindingValidity::Invalid
                        }
                    }
                }
            }
            // Every named column must exist; one missing makes the fact
            // invalid. Matching is case-insensitive, the convention the column
            // binding and the fingerprint use, so a renamed-only-column still
            // resolves and is detected as a change rather than as absence.
            Self::Columns { columns } => {
                for column in columns {
                    if !table
                        .columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(column))
                    {
                        return BindingValidity::Invalid;
                    }
                }
                BindingValidity::Valid
            }
        }
    }

    /// Validates this binding against a table found inside a [`SchemaTree`].
    ///
    /// Looks up the table at `(catalog, schema, table)` using case-insensitive
    /// resolution via [`SchemaTree::find_table`]. If found, validates against
    /// that live table; if not found, returns [`BindingValidity::Invalid`].
    pub fn validate_in_tree(
        &self,
        tree: &SchemaTree,
        catalog: &str,
        schema: &str,
        table: &str,
    ) -> BindingValidity {
        match tree.find_table(catalog, schema, table) {
            Some(t) => self.validate(t),
            None => BindingValidity::Invalid,
        }
    }
}

/// Validates a schema binding against an optional live table reference.
///
/// If the table does not exist (`None`), any binding is `Invalid`.
/// If the table exists (`Some(table)`), delegates to [`SchemaBinding::validate`].
pub fn validate_table(binding: &SchemaBinding, table: Option<&Table>) -> BindingValidity {
    match table {
        Some(t) => binding.validate(t),
        None => BindingValidity::Invalid,
    }
}

#[cfg(test)]
#[path = "binding/tests.rs"]
mod tests;
