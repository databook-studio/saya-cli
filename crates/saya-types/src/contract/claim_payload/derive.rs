//! Read-side derivations a reconciler takes from a
//! [`ClaimPayload`](super::ClaimPayload): the stable kind discriminator, the
//! columns a claim depends on, the tombstoned payload `forget` reduces a fact
//! to, and the column snapshots a drift check resolves against a live table.

use super::{ClaimPayload, ReferencedColumn};
use crate::schema::Table;

impl ClaimPayload {
    /// Stable discriminator used as part of a claim's deduplication key.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TableDescription { .. } => "table_description",
            Self::TableAlias { .. } => "table_alias",
            Self::TableGrain { .. } => "table_grain",
            Self::ColumnDescription { .. } => "column_description",
            Self::ColumnRole { .. } => "column_role",
            Self::DefaultTimeColumn { .. } => "default_time_column",
            Self::Relationship { .. } => "relationship",
            Self::JoinRule { .. } => "join_rule",
            Self::MetricDefinition { .. } => "metric_definition",
        }
    }

    /// Columns this claim depends on, for schema-drift reconciliation. Empty for table-level claims.
    pub fn referenced_columns(&self) -> Vec<&str> {
        match self {
            Self::TableDescription { .. } => Vec::new(),
            Self::TableAlias { .. } => Vec::new(),
            Self::TableGrain { .. } => Vec::new(),
            Self::ColumnDescription { column, .. } => vec![column],
            Self::ColumnRole { column, .. } => vec![column],
            Self::DefaultTimeColumn { column, .. } => vec![column],
            Self::Relationship { local_columns, .. } => {
                local_columns.iter().map(|s| s.as_str()).collect()
            }
            // The local join keys live on the table the claim is stored
            // against, so they are the columns a drift check can validate. The
            // target columns belong to a different table and are not tracked
            // here.
            Self::JoinRule { local_columns, .. } => {
                local_columns.iter().map(|s| s.as_str()).collect()
            }
            Self::MetricDefinition { columns, .. } => columns.iter().map(|s| s.as_str()).collect(),
        }
    }

    /// The same variant with its user-derived free text emptied — the payload a
    /// forgotten fact's row is reduced to when `forget` erases its content.
    ///
    /// The deletion promise (`docs/memory.md` §Deletion) is that a forgotten fact's
    /// payload is cleared while the row stays. The secret-bearing channels on a
    /// payload are the free-text fields — `text`, `description`, `alias` — where a
    /// user or the model could paste a credential or a query. Emptying them removes
    /// any user-derived content the row carried. What survives is structural
    /// identity the row needs to stay a meaningful tombstone: the column name on a
    /// column-scoped payload (it is the slot's column, already in the `slot` column
    /// and the row's object identity) and the closed-enum `role` of a `ColumnRole`
    /// (a classification, not free text — and one of a fixed five values, so it
    /// carries nothing a user supplied). A `Relationship` matches no slot and is
    /// never stored, so it cannot reach `forget`; it is returned unchanged as a
    /// total-function defence, not a case the store exercises.
    ///
    /// The result is the same variant, so `slot_matches_payload` still holds and
    /// a read that decodes a dismissed row does not fail. Constructing it here
    /// directly (rather than via the validating constructors) is correct: the
    /// constructors *reject* empty text, and the point of a blanked payload is
    /// exactly to carry none.
    pub fn blanked(&self) -> Self {
        match self {
            Self::TableDescription { .. } => Self::TableDescription {
                text: String::new(),
            },
            Self::TableAlias { .. } => Self::TableAlias {
                alias: String::new(),
            },
            Self::TableGrain { .. } => Self::TableGrain {
                description: String::new(),
                reason: None,
            },
            Self::ColumnDescription { column, .. } => Self::ColumnDescription {
                column: column.clone(),
                text: String::new(),
            },
            // `role` is a closed enum, not user free text; `column` is the slot's
            // structural column. Neither is a secret-bearing channel, so both stay.
            Self::ColumnRole { column, role, .. } => Self::ColumnRole {
                column: column.clone(),
                role: *role,
                reason: None,
            },
            Self::DefaultTimeColumn { column, .. } => Self::DefaultTimeColumn {
                column: column.clone(),
                reason: None,
            },
            // `condition` is the free-text body a user or model could paste a
            // credential into, so it is emptied; `reason` is free text too. The
            // target and the join keys are structural identity — column names
            // and a qualified target, reconstructable and not a secret-bearing
            // channel — so a tombstone keeps them.
            Self::JoinRule {
                target,
                local_columns,
                target_columns,
                ..
            } => Self::JoinRule {
                target: target.clone(),
                local_columns: local_columns.clone(),
                target_columns: target_columns.clone(),
                condition: String::new(),
                reason: None,
            },
            // `definition` is the free-text formula; `name` and `columns` are
            // the structural identity of which metric on which columns, so a
            // tombstone keeps them and the row stays a meaningful marker.
            Self::MetricDefinition {
                name,
                definition: _,
                columns,
                ..
            } => Self::MetricDefinition {
                name: name.clone(),
                definition: String::new(),
                columns: columns.clone(),
                reason: None,
            },
            // Unreachable via the store (no slot names a relationship), so this
            // arm only keeps `blanked` total. Returning it unchanged preserves
            // the constructor-validated invariants rather than fabricating a
            // malformed relationship.
            Self::Relationship { .. } => self.clone(),
        }
    }

    /// Snapshots of this claim's referenced columns, resolved against `table`.
    /// A referenced column absent from `table` is skipped — a claim cannot
    /// snapshot what does not exist, and the caller decides what that means.
    /// Name matching is case-insensitive, the same convention the schema
    /// fingerprint and the validity reconciler use, so a column that kept its
    /// name but changed type resolves here and is detected as a change later.
    pub fn referenced_column_snapshots(&self, table: &Table) -> Vec<ReferencedColumn> {
        self.referenced_columns()
            .iter()
            .filter_map(|name| {
                table
                    .columns
                    .iter()
                    .find(|col| col.name.eq_ignore_ascii_case(name))
                    .map(|col| ReferencedColumn {
                        name: name.to_string(),
                        data_type: col.data_type.clone(),
                        nullable: col.nullable,
                    })
            })
            .collect()
    }

    /// Name-only snapshots of this claim's referenced columns, for a caller
    /// that has no live schema to resolve types against. Each referenced
    /// column becomes a snapshot with an empty `data_type` and `nullable:
    /// false`, which the reconciler treats as *unknown* rather than matched —
    /// so a no-schema proposal still records the column names a later drift
    /// rule needs (a removed referenced column reads as Stale), without ever
    /// claiming a type it never observed.
    pub fn referenced_column_name_snapshots(&self) -> Vec<ReferencedColumn> {
        self.referenced_columns()
            .into_iter()
            .map(|name| ReferencedColumn {
                name: name.to_string(),
                data_type: String::new(),
                nullable: false,
            })
            .collect()
    }
}
