use serde::{Deserialize, Serialize};

use crate::contract::claim::{MAX_REFERENCED_COLUMNS, validate_text};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::error::ContractError;
use crate::contract::identity::{DatabaseObjectRef, validate_name};
use crate::schema::Table;

/// A referenced column snapshotted at claim time: its name plus the type and
/// nullability the connector reported when the claim was made.
///
/// Phase 5a persists this so a later schema change can tell a *retyped*
/// referenced column (the claim may now be wrong) from an *unrelated* column
/// changing elsewhere (the claim is fine). A claim proposed without a live
/// table stores no snapshot; `data_type` is then empty and `nullable` is
/// `false`, which the reconciler treats as "unknown" rather than "matched".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferencedColumn {
    pub name: String,
    /// The column's type as the connector reported it when the claim was made.
    /// Empty for old `["a","b"]`-shape rows upgraded in place by the store.
    pub data_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
// Every variant is `#[non_exhaustive]` as well as the enum. Without it another
// crate could write `ClaimPayload::TableDescription { text }` directly and skip
// the validation below, which is the only thing keeping oversized and
// control-character-bearing text out of a rendered context block.
pub enum ClaimPayload {
    #[non_exhaustive]
    TableDescription { text: String },
    #[non_exhaustive]
    TableAlias { alias: String },
    /// A directive claim: what a single row represents. Carries an optional
    /// `reason` so a model that would otherwise argue with the grain reads the
    /// justification (spec: claim-reasons). The reason is validated free text,
    /// bounded and control-char-stripped the way `TableDescription`'s `text`
    /// is; `None` is the state of every claim written before the field existed.
    #[non_exhaustive]
    TableGrain {
        description: String,
        #[serde(default)]
        reason: Option<String>,
    },
    #[non_exhaustive]
    ColumnDescription { column: String, text: String },
    /// A directive claim: the role a column plays. Carries an optional `reason`
    /// (see [`Self::TableGrain`]).
    #[non_exhaustive]
    ColumnRole {
        column: String,
        role: ColumnRole,
        #[serde(default)]
        reason: Option<String>,
    },
    /// A directive claim: the column to use as the default time dimension.
    /// Carries an optional `reason` (see [`Self::TableGrain`]).
    #[non_exhaustive]
    DefaultTimeColumn {
        column: String,
        #[serde(default)]
        reason: Option<String>,
    },
    #[non_exhaustive]
    Relationship {
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    },
    /// A join condition the user taught, stored against the local table. The
    /// joined table is `target` (a qualified name in the same profile as the
    /// claim's object); `local_columns` and `target_columns` are the paired
    /// equi-join keys (both empty when the rule is a predicate-only join no
    /// declared constraint describes). `condition` is the full join condition
    /// as free text — it carries the extra predicate or soft-delete filter a
    /// real join adds on top of the keys, and is the only field a model or user
    /// could paste a credential into, so `blanked` empties it.
    #[non_exhaustive]
    JoinRule {
        target: String,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        condition: String,
        #[serde(default)]
        reason: Option<String>,
    },
    /// A business metric defined over the table the claim is stored against.
    /// `name` is the metric's handle, `definition` the formula as free text,
    /// and `columns` the underlying columns the binding tracks so the fact is
    /// invalidated when one of them disappears. `definition` is the
    /// secret-bearing field; `name` and `columns` are structural identity a
    /// tombstone keeps.
    #[non_exhaustive]
    MetricDefinition {
        name: String,
        definition: String,
        columns: Vec<String>,
        #[serde(default)]
        reason: Option<String>,
    },
}

/// Validates an optional reason on a directive claim. A reason is free text a
/// user or model supplied, so it is bounded and control-char-stripped exactly
/// the way [`validate_text`] bounds a description — a reason is a claim-shaped
/// field, not an unbounded annotation. `None` passes through: no reason is the
/// state of every directive claim written before the field existed, and the
/// common case where only the value is stated. An empty or whitespace-only
/// reason carries no information, so it collapses to `None` rather than
/// erroring — the field is optional, and a user who typed only spaces meant
/// none.
fn validate_reason(reason: Option<&str>) -> Result<Option<String>, ContractError> {
    match reason.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => {
            validate_text(text)?;
            Ok(Some(text.to_string()))
        }
        None => Ok(None),
    }
}

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

    pub fn table_description(text: impl Into<String>) -> Result<Self, ContractError> {
        let text = text.into();
        validate_text(&text)?;
        Ok(Self::TableDescription { text })
    }

    pub fn table_alias(alias: impl Into<String>) -> Result<Self, ContractError> {
        let alias = alias.into();
        validate_name(&alias)?;
        Ok(Self::TableAlias { alias })
    }

    pub fn table_grain(
        description: impl Into<String>,
        reason: Option<&str>,
    ) -> Result<Self, ContractError> {
        let description = description.into();
        validate_text(&description)?;
        let reason = validate_reason(reason)?;
        Ok(Self::TableGrain {
            description,
            reason,
        })
    }

    pub fn column_description(
        column: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let column = column.into();
        let text = text.into();
        validate_name(&column)?;
        validate_text(&text)?;
        Ok(Self::ColumnDescription { column, text })
    }

    pub fn column_role(
        column: impl Into<String>,
        role: ColumnRole,
        reason: Option<&str>,
    ) -> Result<Self, ContractError> {
        let column = column.into();
        validate_name(&column)?;
        let reason = validate_reason(reason)?;
        Ok(Self::ColumnRole {
            column,
            role,
            reason,
        })
    }

    pub fn default_time_column(
        column: impl Into<String>,
        reason: Option<&str>,
    ) -> Result<Self, ContractError> {
        let column = column.into();
        validate_name(&column)?;
        let reason = validate_reason(reason)?;
        Ok(Self::DefaultTimeColumn { column, reason })
    }

    pub fn relationship(
        target: DatabaseObjectRef,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        cardinality: Cardinality,
    ) -> Result<Self, ContractError> {
        if local_columns.is_empty() || target_columns.is_empty() {
            return Err(ContractError::EmptyColumns);
        }
        if local_columns.len() != target_columns.len() {
            return Err(ContractError::ColumnCountMismatch);
        }
        if local_columns.len() > MAX_REFERENCED_COLUMNS {
            return Err(ContractError::TooManyColumns);
        }
        for col in local_columns.iter().chain(target_columns.iter()) {
            validate_name(col)?;
        }
        Ok(Self::Relationship {
            target,
            local_columns,
            target_columns,
            cardinality,
        })
    }

    /// Builds a `JoinRule` payload. The target is a qualified name in the same
    /// profile as the claim's object. Both column lists may be empty — a join
    /// can cross tables with no declared constraint, so the rule may carry only
    /// the free-text `condition` — but when either list is non-empty the two
    /// must pair positionally and each name is validated. The condition is the
    /// full join condition as bounded, control-char-free text; the optional
    /// reason is validated the way a directive claim's reason is.
    pub fn join_rule(
        target: impl Into<String>,
        local_columns: Vec<String>,
        target_columns: Vec<String>,
        condition: impl Into<String>,
        reason: Option<&str>,
    ) -> Result<Self, ContractError> {
        let target = target.into();
        validate_name(&target)?;
        let condition = condition.into();
        validate_text(&condition)?;
        if local_columns.len() != target_columns.len() {
            return Err(ContractError::ColumnCountMismatch);
        }
        if local_columns.len() > MAX_REFERENCED_COLUMNS {
            return Err(ContractError::TooManyColumns);
        }
        for col in local_columns.iter().chain(target_columns.iter()) {
            validate_name(col)?;
        }
        let reason = validate_reason(reason)?;
        Ok(Self::JoinRule {
            target,
            local_columns,
            target_columns,
            condition,
            reason,
        })
    }

    /// Builds a `MetricDefinition` payload. The name is a short handle
    /// (validated as a name), the definition is the formula as bounded,
    /// control-char-free text, and `columns` are the underlying columns the
    /// binding tracks. `columns` may be empty — a metric with no column
    /// references binds to the table existing — but each named column is
    /// validated and the count is bounded.
    pub fn metric_definition(
        name: impl Into<String>,
        definition: impl Into<String>,
        columns: Vec<String>,
        reason: Option<&str>,
    ) -> Result<Self, ContractError> {
        let name = name.into();
        validate_name(&name)?;
        let definition = definition.into();
        validate_text(&definition)?;
        if columns.len() > MAX_REFERENCED_COLUMNS {
            return Err(ContractError::TooManyColumns);
        }
        for col in columns.iter() {
            validate_name(col)?;
        }
        let reason = validate_reason(reason)?;
        Ok(Self::MetricDefinition {
            name,
            definition,
            columns,
            reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::claim::{CLAIM_PAYLOAD_VERSION, MAX_TEXT_CHARS};
    use crate::contract::claim_enums::{Cardinality, ColumnRole};
    use crate::{DatabaseObjectKind, ProfileIdentity};

    fn make_profile() -> ProfileIdentity {
        ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn make_target() -> DatabaseObjectRef {
        let profile = make_profile();
        DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
    }

    #[test]
    fn constructors_reject_empty_text() {
        assert!(ClaimPayload::table_description("").is_err());
        assert!(ClaimPayload::table_grain("", None).is_err());
        assert!(ClaimPayload::column_description("col", "").is_err());
    }

    #[test]
    fn constructors_reject_long_text() {
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(ClaimPayload::table_description(&long).is_err());
        assert!(ClaimPayload::table_grain(&long, None).is_err());
        assert!(ClaimPayload::column_description("col", &long).is_err());
    }

    #[test]
    fn constructors_reject_control_chars() {
        assert!(ClaimPayload::table_description("hello\nworld").is_err());
        assert!(ClaimPayload::table_grain("hello\tworld", None).is_err());
        assert!(ClaimPayload::column_description("col", "text\n").is_err());
    }

    #[test]
    fn table_alias_rejects_empty() {
        assert!(ClaimPayload::table_alias("").is_err());
    }

    #[test]
    fn column_role_validates_column_name() {
        assert!(ClaimPayload::column_role("", ColumnRole::Dimension, None).is_err());
        assert!(ClaimPayload::column_role("col\n", ColumnRole::Dimension, None).is_err());
    }

    #[test]
    fn default_time_column_validates_column_name() {
        assert!(ClaimPayload::default_time_column("", None).is_err());
    }

    // --- The reason a directive claim carries (spec: claim-reasons). ---
    //
    // A directive claim (`DefaultTimeColumn`, `TableGrain`, `ColumnRole`) may
    // carry the *why* alongside the *what*, so a model that would otherwise
    // argue with it reads the justification. The reason is optional, free text,
    // bounded and validated exactly the way `TableDescription`'s text is
    // (`validate_text`: non-empty, ≤ MAX_TEXT_CHARS, no control characters).

    #[test]
    fn default_time_column_carries_an_optional_reason() {
        // No reason: the directive claims a user made before this change, and
        // the common case where only the value is stated.
        let none = ClaimPayload::default_time_column("return_date", None).unwrap();
        assert!(matches!(
            none,
            ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
        ));
        let with = ClaimPayload::default_time_column(
            "return_date",
            Some("a rental only counts once it comes back"),
        )
        .unwrap();
        assert!(matches!(
            with,
            ClaimPayload::DefaultTimeColumn { reason: Some(r), .. } if r == "a rental only counts once it comes back"
        ));
    }

    #[test]
    fn table_grain_carries_an_optional_reason() {
        let none = ClaimPayload::table_grain("one row per order", None).unwrap();
        assert!(matches!(
            none,
            ClaimPayload::TableGrain { reason: None, .. }
        ));
        let with =
            ClaimPayload::table_grain("one row per order", Some("orders ship separately")).unwrap();
        assert!(matches!(
            with,
            ClaimPayload::TableGrain { reason: Some(r), .. } if r == "orders ship separately"
        ));
    }

    #[test]
    fn column_role_carries_an_optional_reason() {
        let none = ClaimPayload::column_role("amount", ColumnRole::Measure, None).unwrap();
        assert!(matches!(
            none,
            ClaimPayload::ColumnRole { reason: None, .. }
        ));
        let with = ClaimPayload::column_role(
            "amount",
            ColumnRole::Measure,
            Some("money the customer paid"),
        )
        .unwrap();
        assert!(matches!(
            with,
            ClaimPayload::ColumnRole { reason: Some(r), .. } if r == "money the customer paid"
        ));
    }

    #[test]
    fn reason_is_validated_like_description_text() {
        // Too long and control characters are rejected by the same
        // `validate_text` bound `TableDescription` uses — a reason is a
        // claim-shaped free-text field, not an unbounded annotation. An empty
        // or whitespace-only reason carries no information, so it collapses to
        // `None` (no reason) rather than erroring: the field is optional, and a
        // user who typed only spaces meant none.
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(ClaimPayload::default_time_column("c", Some(&long)).is_err());
        assert!(ClaimPayload::table_grain("g", Some(&long)).is_err());
        assert!(ClaimPayload::column_role("c", ColumnRole::Measure, Some(&long)).is_err());

        assert!(ClaimPayload::default_time_column("c", Some("line\nbreak")).is_err());
        assert!(ClaimPayload::table_grain("g", Some("tab\there")).is_err());
        assert!(ClaimPayload::column_role("c", ColumnRole::Measure, Some("ctl\0char")).is_err());

        // An empty or whitespace-only reason collapses to `None` — no error, no
        // reason stored. The value is still constructed.
        let empty = ClaimPayload::default_time_column("c", Some("   ")).unwrap();
        assert!(matches!(
            empty,
            ClaimPayload::DefaultTimeColumn { reason: None, .. }
        ));
        let blank = ClaimPayload::table_grain("g", Some("")).unwrap();
        assert!(matches!(
            blank,
            ClaimPayload::TableGrain { reason: None, .. }
        ));
    }

    /// An old payload written before the reason field exists must decode back
    /// as `reason: None`, not fail. A decode failure on the read path would
    /// take out a user's entire memory, and a missing reason is the state of
    /// every claim written before this change. The JSON is hand-written, not
    /// generated by the current serializer — a round-trip proves nothing about
    /// the old format.
    #[test]
    fn old_payload_without_reason_decodes_as_none() {
        let old_default_time = r#"{"kind":"default_time_column","column":"return_date"}"#;
        let p: ClaimPayload = serde_json::from_str(old_default_time).unwrap();
        assert!(matches!(
            p,
            ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
        ));

        let old_grain = r#"{"kind":"table_grain","description":"one row per order"}"#;
        let p: ClaimPayload = serde_json::from_str(old_grain).unwrap();
        assert!(matches!(
            p,
            ClaimPayload::TableGrain { ref description, ref reason } if description == "one row per order" && reason.is_none()
        ));

        let old_role = r#"{"kind":"column_role","column":"amount","role":"measure"}"#;
        let p: ClaimPayload = serde_json::from_str(old_role).unwrap();
        assert!(matches!(
            p,
            ClaimPayload::ColumnRole { ref column, role: ColumnRole::Measure, ref reason } if column == "amount" && reason.is_none()
        ));
    }

    #[test]
    fn relationship_rejects_empty_column_lists() {
        let target = make_target();
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec![],
                vec!["b".into()],
                Cardinality::OneToOne
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec!["a".into()],
                vec![],
                Cardinality::OneToOne
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target,
                vec!["a".into()],
                vec!["b".into()],
                Cardinality::OneToOne
            )
            .is_ok()
        );
    }

    #[test]
    fn relationship_rejects_mismatched_lengths() {
        let target = make_target();
        let result = ClaimPayload::relationship(
            target,
            vec!["a".into(), "b".into()],
            vec!["x".into()],
            Cardinality::ManyToOne,
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ContractError::ColumnCountMismatch);
    }

    #[test]
    fn relationship_rejects_too_many_columns() {
        let target = make_target();
        let cols: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
            .map(|i| format!("c{i}"))
            .collect();
        let result = ClaimPayload::relationship(target, cols.clone(), cols, Cardinality::OneToMany);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ContractError::TooManyColumns);
    }

    #[test]
    fn relationship_validates_column_names() {
        let target = make_target();
        assert!(
            ClaimPayload::relationship(
                target.clone(),
                vec!["".into()],
                vec!["x".into()],
                Cardinality::OneToOne,
            )
            .is_err()
        );
        assert!(
            ClaimPayload::relationship(
                target,
                vec!["x".into()],
                vec!["".into()],
                Cardinality::OneToOne,
            )
            .is_err()
        );
    }

    #[test]
    fn join_rule_rejects_empty_or_bad_condition_and_target() {
        // The condition is the free-text body; an empty one carries no fact.
        assert!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["customer_id".into()],
                vec!["id".into()],
                "",
                None
            )
            .is_err()
        );
        // A control character in the condition is rejected, not scrubbed.
        assert!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["customer_id".into()],
                vec!["id".into()],
                "join\nhere",
                None
            )
            .is_err()
        );
        // An empty target names no relation.
        assert!(
            ClaimPayload::join_rule(
                "",
                vec!["customer_id".into()],
                vec!["id".into()],
                "orders.customer_id = customers.id",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn join_rule_pairs_columns_and_bounds_the_count() {
        // Mismatched lengths break the positional pairing a join key list needs.
        assert_eq!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["a".into(), "b".into()],
                vec!["x".into()],
                "cond",
                None
            )
            .unwrap_err(),
            ContractError::ColumnCountMismatch
        );
        // Both lists may be empty — a predicate-only join with no declared keys.
        assert!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec![],
                vec![],
                "orders joins customers where customers.is_active",
                None
            )
            .is_ok()
        );
        // A bad column name is rejected.
        assert!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["".into()],
                vec!["id".into()],
                "cond",
                None
            )
            .is_err()
        );
        // The column count is bounded.
        let many: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
            .map(|i| format!("c{i}"))
            .collect();
        assert_eq!(
            ClaimPayload::join_rule("catalog.public.customers", many.clone(), many, "cond", None)
                .unwrap_err(),
            ContractError::TooManyColumns
        );
    }

    #[test]
    fn join_rule_carries_an_optional_reason() {
        let none = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            None,
        )
        .unwrap();
        assert!(matches!(none, ClaimPayload::JoinRule { reason: None, .. }));
        let with = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id",
            Some("only active customers count"),
        )
        .unwrap();
        assert!(matches!(
            with,
            ClaimPayload::JoinRule { reason: Some(r), .. } if r == "only active customers count"
        ));
    }

    #[test]
    fn metric_definition_rejects_empty_name_and_definition() {
        assert!(ClaimPayload::metric_definition("", "SUM(amount)", vec![], None).is_err());
        assert!(ClaimPayload::metric_definition("mrr", "", vec![], None).is_err());
        assert!(ClaimPayload::metric_definition("mrr", "SUM(amount)\n", vec![], None).is_err());
    }

    #[test]
    fn metric_definition_bounds_and_validates_columns() {
        assert!(
            ClaimPayload::metric_definition("mrr", "SUM(amount)", vec!["".into()], None).is_err()
        );
        let many: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
            .map(|i| format!("c{i}"))
            .collect();
        assert_eq!(
            ClaimPayload::metric_definition("mrr", "SUM(amount)", many, None).unwrap_err(),
            ContractError::TooManyColumns
        );
        // An empty column list is allowed: a metric with no column references
        // binds to the table existing.
        assert!(ClaimPayload::metric_definition("count", "COUNT(*)", vec![], None).is_ok());
    }

    #[test]
    fn metric_definition_carries_an_optional_reason() {
        let none = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into()],
            None,
        )
        .unwrap();
        assert!(matches!(
            none,
            ClaimPayload::MetricDefinition { reason: None, .. }
        ));
        let with = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into()],
            Some("recurring revenue only"),
        )
        .unwrap();
        assert!(matches!(
            with,
            ClaimPayload::MetricDefinition { reason: Some(r), .. } if r == "recurring revenue only"
        ));
    }

    /// A join rule or metric written before the reason field exists must decode
    /// with `reason: None`, not fail — a decode failure on the read path would
    /// take out a user's entire memory. The JSON is hand-written to prove the
    /// old shape loads, not just that the current serializer round-trips.
    #[test]
    fn old_join_rule_and_metric_without_reason_decode_as_none() {
        let old_join = r#"{"kind":"join_rule","target":"catalog.public.customers","local_columns":["customer_id"],"target_columns":["id"],"condition":"orders.customer_id = customers.id"}"#;
        let p: ClaimPayload = serde_json::from_str(old_join).unwrap();
        assert!(matches!(
            p,
            ClaimPayload::JoinRule { ref target, ref condition, ref reason, .. }
                if target == "catalog.public.customers"
                    && condition == "orders.customer_id = customers.id"
                    && reason.is_none()
        ));

        let old_metric = r#"{"kind":"metric_definition","name":"mrr","definition":"SUM(subscription_amount) WHERE status = 'active'","columns":["subscription_amount","status"]}"#;
        let p: ClaimPayload = serde_json::from_str(old_metric).unwrap();
        assert!(matches!(
            p,
            ClaimPayload::MetricDefinition { ref name, ref definition, ref columns, ref reason }
                if name == "mrr"
                    && definition == "SUM(subscription_amount) WHERE status = 'active'"
                    && columns.as_slice() == ["subscription_amount", "status"]
                    && reason.is_none()
        ));
    }

    #[test]
    fn join_rule_and_metric_serde_round_trip() {
        let join = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id and customers.is_active",
            Some("only active customers count"),
        )
        .unwrap();
        let json = serde_json::to_string(&join).unwrap();
        let back: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(join, back);

        let metric = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            Some("recurring revenue only"),
        )
        .unwrap();
        let json = serde_json::to_string(&metric).unwrap();
        let back: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(metric, back);
    }

    #[test]
    fn blanked_empties_join_rule_condition_but_keeps_keys_and_target() {
        let join = ClaimPayload::join_rule(
            "catalog.public.customers",
            vec!["customer_id".into()],
            vec!["id".into()],
            "orders.customer_id = customers.id and customers.is_active",
            Some("only active customers count"),
        )
        .unwrap();
        let blanked = join.blanked();
        assert!(matches!(
            blanked,
            ClaimPayload::JoinRule {
                ref target,
                ref local_columns,
                ref target_columns,
                ref condition,
                ref reason,
            } if target == "catalog.public.customers"
                && local_columns.as_slice() == ["customer_id"]
                && target_columns.as_slice() == ["id"]
                && condition.is_empty()
                && reason.is_none()
        ));
        // The blanked payload round-trips through serde so a later read decodes.
        let json = serde_json::to_string(&blanked).unwrap();
        let back: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, blanked);
    }

    #[test]
    fn blanked_empties_metric_definition_but_keeps_name_and_columns() {
        let metric = ClaimPayload::metric_definition(
            "mrr",
            "SUM(subscription_amount) WHERE status = 'active'",
            vec!["subscription_amount".into(), "status".into()],
            Some("recurring revenue only"),
        )
        .unwrap();
        let blanked = metric.blanked();
        assert!(matches!(
            blanked,
            ClaimPayload::MetricDefinition {
                ref name,
                ref definition,
                ref columns,
                ref reason,
            } if name == "mrr"
                && definition.is_empty()
                && columns.as_slice() == ["subscription_amount", "status"]
                && reason.is_none()
        ));
        let json = serde_json::to_string(&blanked).unwrap();
        let back: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back, blanked);
    }

    #[test]
    fn kind_discriminator() {
        let target = make_target();
        assert_eq!(
            ClaimPayload::table_description("desc").unwrap().kind(),
            "table_description"
        );
        assert_eq!(
            ClaimPayload::table_alias("t").unwrap().kind(),
            "table_alias"
        );
        assert_eq!(
            ClaimPayload::table_grain("g", None).unwrap().kind(),
            "table_grain"
        );
        assert_eq!(
            ClaimPayload::column_description("c", "d").unwrap().kind(),
            "column_description"
        );
        assert_eq!(
            ClaimPayload::column_role("c", ColumnRole::Identifier, None)
                .unwrap()
                .kind(),
            "column_role"
        );
        assert_eq!(
            ClaimPayload::default_time_column("c", None).unwrap().kind(),
            "default_time_column"
        );
        assert_eq!(
            ClaimPayload::relationship(
                target,
                vec!["a".into()],
                vec!["b".into()],
                Cardinality::OneToMany
            )
            .unwrap()
            .kind(),
            "relationship"
        );
        assert_eq!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["customer_id".into()],
                vec!["id".into()],
                "orders.customer_id = customers.id",
                None
            )
            .unwrap()
            .kind(),
            "join_rule"
        );
        assert_eq!(
            ClaimPayload::metric_definition(
                "mrr",
                "SUM(subscription_amount) WHERE status = 'active'",
                vec!["subscription_amount".into(), "status".into()],
                None
            )
            .unwrap()
            .kind(),
            "metric_definition"
        );
    }

    #[test]
    fn referenced_columns() {
        let target = make_target();
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert!(
            ClaimPayload::table_alias("t")
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert!(
            ClaimPayload::table_grain("g", None)
                .unwrap()
                .referenced_columns()
                .is_empty()
        );
        assert_eq!(
            ClaimPayload::column_description("c", "d")
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::column_role("c", ColumnRole::Measure, None)
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::default_time_column("c", None)
                .unwrap()
                .referenced_columns(),
            vec!["c"]
        );
        assert_eq!(
            ClaimPayload::relationship(
                target,
                vec!["local_a".into(), "local_b".into()],
                vec!["tgt_a".into(), "tgt_b".into()],
                Cardinality::ManyToMany,
            )
            .unwrap()
            .referenced_columns(),
            vec!["local_a", "local_b"]
        );
        // A join rule depends on its local join keys — the columns on the
        // table the claim is stored against — never the target's columns.
        assert_eq!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["customer_id".into(), "tenant_id".into()],
                vec!["id".into(), "tenant".into()],
                "orders.customer_id = customers.id",
                None
            )
            .unwrap()
            .referenced_columns(),
            vec!["customer_id", "tenant_id"]
        );
        // A predicate-only join with no equi-join keys depends on nothing the
        // local table's columns can name, so its referenced columns are empty.
        assert!(
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec![],
                vec![],
                "orders joins customers where customers.is_active",
                None
            )
            .unwrap()
            .referenced_columns()
            .is_empty()
        );
        assert_eq!(
            ClaimPayload::metric_definition(
                "mrr",
                "SUM(subscription_amount) WHERE status = 'active'",
                vec!["subscription_amount".into(), "status".into()],
                None
            )
            .unwrap()
            .referenced_columns(),
            vec!["subscription_amount", "status"]
        );
    }

    #[test]
    fn serde_round_trip() {
        let payload = ClaimPayload::table_description("hello world").unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: ClaimPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload, deserialized);
    }

    #[test]
    fn blanked_empties_free_text_but_keeps_structural_identity() {
        // The secret-bearing free-text fields are cleared to empty.
        let desc = ClaimPayload::table_description("a customers table").unwrap();
        assert_eq!(
            desc.blanked(),
            ClaimPayload::TableDescription {
                text: String::new()
            }
        );
        let alias = ClaimPayload::table_alias("customers").unwrap();
        assert_eq!(
            alias.blanked(),
            ClaimPayload::TableAlias {
                alias: String::new()
            }
        );
        let grain = ClaimPayload::table_grain("one row per order", None).unwrap();
        assert_eq!(
            grain.blanked(),
            ClaimPayload::TableGrain {
                description: String::new(),
                reason: None,
            }
        );
        // A column-scoped description keeps its column (the slot's structural
        // column, not free text) but drops the descriptive text.
        let col_desc =
            ClaimPayload::column_description("created_at", "when the row was made").unwrap();
        assert_eq!(
            col_desc.blanked(),
            ClaimPayload::ColumnDescription {
                column: "created_at".into(),
                text: String::new(),
            }
        );
        // A column role keeps the column and the closed-enum role — neither is
        // user free text, and both are reconstructable from the slot.
        let col_role =
            ClaimPayload::column_role("created_at", ColumnRole::Timestamp, None).unwrap();
        assert_eq!(
            col_role.blanked(),
            ClaimPayload::ColumnRole {
                column: "created_at".into(),
                role: ColumnRole::Timestamp,
                reason: None,
            }
        );
        let default_time = ClaimPayload::default_time_column("created_at", None).unwrap();
        assert_eq!(
            default_time.blanked(),
            ClaimPayload::DefaultTimeColumn {
                column: "created_at".into(),
                reason: None,
            }
        );
        // A directive claim's reason is user-supplied free text — a secret-bearing
        // channel like `description`/`alias` — so `forget` blanks it too. A
        // forgotten tombstone must not leak the justification a user wrote.
        let reasoned = ClaimPayload::default_time_column(
            "return_date",
            Some("a rental only counts once it comes back"),
        )
        .unwrap();
        assert!(matches!(
            reasoned.blanked(),
            ClaimPayload::DefaultTimeColumn { ref column, ref reason } if column == "return_date" && reason.is_none()
        ));
        let grain_reason = ClaimPayload::table_grain("one row per order", Some("why")).unwrap();
        assert!(matches!(
            grain_reason.blanked(),
            ClaimPayload::TableGrain { ref reason, .. } if reason.is_none()
        ));
        let role_reason =
            ClaimPayload::column_role("amount", ColumnRole::Measure, Some("why")).unwrap();
        assert!(matches!(
            role_reason.blanked(),
            ClaimPayload::ColumnRole { ref reason, .. } if reason.is_none()
        ));
    }

    #[test]
    fn blanked_is_the_same_variant_and_serializes_so_reads_do_not_break() {
        // A forgotten row's `value_json` must still decode as a `ClaimPayload` or
        // every read of its object fails. `blanked` keeps the variant, so the
        // serialised form round-trips through serde.
        for payload in [
            ClaimPayload::table_description("d").unwrap(),
            ClaimPayload::table_alias("a").unwrap(),
            ClaimPayload::table_grain("g", None).unwrap(),
            ClaimPayload::column_description("c", "d").unwrap(),
            ClaimPayload::column_role("c", ColumnRole::Measure, None).unwrap(),
            ClaimPayload::default_time_column("c", None).unwrap(),
            ClaimPayload::join_rule(
                "catalog.public.customers",
                vec!["customer_id".into()],
                vec!["id".into()],
                "orders.customer_id = customers.id",
                None,
            )
            .unwrap(),
            ClaimPayload::metric_definition(
                "mrr",
                "SUM(subscription_amount) WHERE status = 'active'",
                vec!["subscription_amount".into()],
                None,
            )
            .unwrap(),
        ] {
            let blanked = payload.blanked();
            assert_eq!(blanked.kind(), payload.kind(), "blanked keeps the variant");
            let json = serde_json::to_string(&blanked).unwrap();
            let back: ClaimPayload = serde_json::from_str(&json).unwrap();
            assert_eq!(back, blanked, "blanked payload round-trips through serde");
        }
    }

    fn table_with(cols: &[(&str, &str, bool)]) -> crate::Table {
        crate::Table {
            name: "t".into(),
            columns: cols
                .iter()
                .map(|(name, ty, nullable)| crate::Column {
                    name: (*name).into(),
                    data_type: (*ty).into(),
                    nullable: *nullable,
                })
                .collect(),
            primary_key: vec![],
            foreign_keys: vec![],
        }
    }

    #[test]
    fn snapshots_resolve_type_and_nullability_from_the_live_table() {
        let table = table_with(&[("user_id", "bigint", false), ("amount", "numeric", true)]);
        let payload = ClaimPayload::column_description("user_id", "the user id").unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }]
        );
    }

    #[test]
    fn snapshot_skips_a_referenced_column_absent_from_the_table() {
        // A claim cannot snapshot what does not exist; the caller decides what
        // that means. The helper returns only the columns it could resolve.
        let table = table_with(&[("id", "bigint", false)]);
        let payload = ClaimPayload::column_description("missing", "gone").unwrap();
        assert!(payload.referenced_column_snapshots(&table).is_empty());
    }

    #[test]
    fn snapshots_match_case_insensitively_like_fingerprinting_does() {
        // Schema discovery and fingerprint comparison are case-insensitive on
        // object and column names; a snapshot must resolve the same way or a
        // retyped column would read as "absent" instead of "changed".
        let table = table_with(&[("User_Id", "bigint", false)]);
        let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }]
        );
    }

    #[test]
    fn relationship_snapshots_all_local_columns() {
        let table = table_with(&[
            ("local_a", "int", false),
            ("local_b", "text", true),
            ("unrelated", "int", false),
        ]);
        let target = make_target();
        let payload = ClaimPayload::relationship(
            target,
            vec!["local_a".into(), "local_b".into()],
            vec!["x".into(), "y".into()],
            Cardinality::OneToMany,
        )
        .unwrap();
        assert_eq!(
            payload.referenced_column_snapshots(&table),
            vec![
                ReferencedColumn {
                    name: "local_a".into(),
                    data_type: "int".into(),
                    nullable: false,
                },
                ReferencedColumn {
                    name: "local_b".into(),
                    data_type: "text".into(),
                    nullable: true,
                },
            ]
        );
    }

    #[test]
    fn table_level_claims_snapshot_nothing() {
        let table = table_with(&[("id", "bigint", false)]);
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_column_snapshots(&table)
                .is_empty()
        );
        assert!(
            ClaimPayload::table_alias("a")
                .unwrap()
                .referenced_column_snapshots(&table)
                .is_empty()
        );
    }

    #[test]
    fn name_only_snapshots_record_names_without_claiming_a_type() {
        // A no-schema caller records the column names a drift rule needs, but
        // with an empty type so the reconciler treats them as unknown.
        let payload = ClaimPayload::column_description("user_id", "the id").unwrap();
        assert_eq!(
            payload.referenced_column_name_snapshots(),
            vec![ReferencedColumn {
                name: "user_id".into(),
                data_type: String::new(),
                nullable: false,
            }]
        );
        assert!(
            ClaimPayload::table_description("desc")
                .unwrap()
                .referenced_column_name_snapshots()
                .is_empty()
        );
    }

    #[test]
    fn referenced_column_serde_round_trips() {
        let col = ReferencedColumn {
            name: "amount".into(),
            data_type: "numeric".into(),
            nullable: true,
        };
        let json = serde_json::to_string(&col).unwrap();
        let back: ReferencedColumn = serde_json::from_str(&json).unwrap();
        assert_eq!(col, back);
    }

    #[test]
    fn claim_payload_version_is_three() {
        // claim-reasons adds an optional `reason` to the directive variants.
        // The field is `#[serde(default)]`, so a version-2 row decodes with
        // `reason: None`; no migration is owed. A claim proposed before this
        // change carries version 2 (or 1) and must still decode.
        assert_eq!(CLAIM_PAYLOAD_VERSION, 3);
    }
}

#[cfg(test)]
mod property_tests {
    //! Property 5 (spec §1): every `ClaimPayload` constructor either returns a
    //! payload whose text round-trips the input, or a typed error — never a
    //! panic, never a payload containing a control character. Pure. This is the
    //! generalisation of the hand-written constructor tests: the constructors are
    //! the only thing keeping oversized and control-character-bearing text out of
    //! a rendered context block, and a property asks whether that holds across an
    //! input space the hand-picked cases did not imagine.
    use super::*;
    use crate::contract::claim_enums::{Cardinality, ColumnRole};
    use crate::{DatabaseObjectKind, ProfileIdentity};
    use proptest::prelude::*;

    fn make_profile() -> ProfileIdentity {
        ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
    }

    fn make_target() -> DatabaseObjectRef {
        let profile = make_profile();
        DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
    }

    /// Any text a user might supply as a description/alias/grain/value: empty,
    /// long, control chars, unicode — every rejection path must fire, none panic.
    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=1100).prop_map(|chars| chars.into_iter().collect())
    }

    /// Any column name: same wide alphabet, shorter cap (`MAX_NAME_CHARS`).
    fn col_name() -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=130).prop_map(|chars| chars.into_iter().collect())
    }

    /// True when `s` contains any Unicode control character.
    fn has_control(s: &str) -> bool {
        s.chars().any(|c| c.is_control())
    }

    /// All five column roles. `select` needs a `Vec`; an array does not satisfy
    /// `Into<Cow<'static, [_]>>`.
    fn role() -> impl Strategy<Value = ColumnRole> {
        prop::sample::select(vec![
            ColumnRole::Identifier,
            ColumnRole::Dimension,
            ColumnRole::Measure,
            ColumnRole::Timestamp,
            ColumnRole::Sensitive,
        ])
    }

    /// Every cardinality.
    fn cardinality() -> impl Strategy<Value = Cardinality> {
        prop::sample::select(vec![
            Cardinality::OneToOne,
            Cardinality::OneToMany,
            Cardinality::ManyToOne,
            Cardinality::ManyToMany,
        ])
    }

    proptest! {
        /// `table_description`: Ok ⇒ stored text round-trips and has no control
        /// char; Err ⇒ typed error; never panics.
        #[test]
        fn table_description_total(s in text()) {
            match ClaimPayload::table_description(&s) {
                Ok(ClaimPayload::TableDescription { text }) => {
                    prop_assert!(!has_control(&text));
                    prop_assert_eq!(&text, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `table_grain`: same totality contract on its `description` field.
        #[test]
        fn table_grain_total(s in text()) {
            match ClaimPayload::table_grain(&s, None) {
                Ok(ClaimPayload::TableGrain { description, .. }) => {
                    prop_assert!(!has_control(&description));
                    prop_assert_eq!(&description, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `column_description`: Ok ⇒ both `column` and `text` round-trip and hold
        /// no control char; the `column` is a *name* (MAX_NAME_CHARS) and `text`
        /// is a claim body (MAX_TEXT_CHARS), so the two length bounds are separate.
        #[test]
        fn column_description_total(col in col_name(), body in text()) {
            match ClaimPayload::column_description(&col, &body) {
                Ok(ClaimPayload::ColumnDescription { column, text }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert!(!has_control(&text));
                    prop_assert_eq!(&column, &col);
                    prop_assert_eq!(&text, &body);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter
                        | ContractError::EmptyText | ContractError::TextTooLong),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `table_alias`: a name, validated by `validate_name`.
        #[test]
        fn table_alias_total(s in col_name()) {
            match ClaimPayload::table_alias(&s) {
                Ok(ClaimPayload::TableAlias { alias }) => {
                    prop_assert!(!has_control(&alias));
                    prop_assert_eq!(&alias, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `column_role`: a name plus a role; Ok ⇒ name round-trips and has no
        /// control char; the role is echoed back by `referenced_columns`.
        #[test]
        fn column_role_total(col in col_name(), r in role()) {
            match ClaimPayload::column_role(&col, r, None) {
                Ok(ClaimPayload::ColumnRole { column, role, .. }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert_eq!(&column, &col);
                    prop_assert_eq!(role, r);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `default_time_column`: a name.
        #[test]
        fn default_time_column_total(s in col_name()) {
            match ClaimPayload::default_time_column(&s, None) {
                Ok(ClaimPayload::DefaultTimeColumn { column, .. }) => {
                    prop_assert!(!has_control(&column));
                    prop_assert_eq!(&column, &s);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `relationship`: the two column lists and the cardinality round-trip,
        /// every stored column is control-char-free, and the count/length errors
        /// are typed. Generates up to a few columns per side.
        #[test]
        fn relationship_total(
            locals in prop::collection::vec(col_name(), 0..=4),
            targets in prop::collection::vec(col_name(), 0..=4),
            card in cardinality(),
        ) {
            let target = make_target();
            match ClaimPayload::relationship(target, locals.clone(), targets.clone(), card) {
                Ok(ClaimPayload::Relationship {
                    local_columns,
                    target_columns,
                    cardinality,
                    ..
                }) => {
                    for c in local_columns.iter().chain(target_columns.iter()) {
                        prop_assert!(!has_control(c), "control char in relationship column: {c:?}");
                    }
                    prop_assert_eq!(&local_columns, &locals);
                    prop_assert_eq!(&target_columns, &targets);
                    prop_assert_eq!(cardinality, card);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyColumns | ContractError::ColumnCountMismatch
                        | ContractError::TooManyColumns
                        | ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ControlCharacter),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `join_rule`: the target, the column lists and the condition
        /// round-trip, every stored field is control-char-free, and the
        /// count/length/name/condition errors are typed. Generates up to a few
        /// columns per side; both lists may be empty (a predicate-only join).
        #[test]
        fn join_rule_total(
            target in col_name(),
            locals in prop::collection::vec(col_name(), 0..=4),
            targets in prop::collection::vec(col_name(), 0..=4),
            condition in text(),
        ) {
            match ClaimPayload::join_rule(
                &target,
                locals.clone(),
                targets.clone(),
                &condition,
                None,
            ) {
                Ok(ClaimPayload::JoinRule {
                    target: t,
                    local_columns,
                    target_columns,
                    condition: c,
                    ..
                }) => {
                    for col in local_columns.iter().chain(target_columns.iter()) {
                        prop_assert!(!has_control(col), "control char in join column: {col:?}");
                    }
                    prop_assert!(!has_control(&t));
                    prop_assert!(!has_control(&c));
                    prop_assert_eq!(&t, &target);
                    prop_assert_eq!(&c, &condition);
                    prop_assert_eq!(&local_columns, &locals);
                    prop_assert_eq!(&target_columns, &targets);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter
                        | ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::ColumnCountMismatch | ContractError::TooManyColumns),
                    "unexpected error: {e:?}"
                ),
            }
        }

        /// `metric_definition`: the name, definition and columns round-trip,
        /// every stored field is control-char-free, and the name/condition/
        /// column errors are typed. The column list may be empty.
        #[test]
        fn metric_definition_total(
            name in col_name(),
            definition in text(),
            columns in prop::collection::vec(col_name(), 0..=4),
        ) {
            match ClaimPayload::metric_definition(&name, &definition, columns.clone(), None) {
                Ok(ClaimPayload::MetricDefinition {
                    name: n,
                    definition: d,
                    columns: cols,
                    ..
                }) => {
                    for col in cols.iter() {
                        prop_assert!(!has_control(col), "control char in metric column: {col:?}");
                    }
                    prop_assert!(!has_control(&n));
                    prop_assert!(!has_control(&d));
                    prop_assert_eq!(&n, &name);
                    prop_assert_eq!(&d, &definition);
                    prop_assert_eq!(&cols, &columns);
                }
                Ok(other) => prop_assert!(false, "wrong variant: {other:?}"),
                Err(e) => prop_assert!(
                    matches!(e,
                        ContractError::EmptyText | ContractError::TextTooLong
                        | ContractError::ControlCharacter
                        | ContractError::EmptyName | ContractError::NameTooLong
                        | ContractError::TooManyColumns),
                    "unexpected error: {e:?}"
                ),
            }
        }
    }
}
