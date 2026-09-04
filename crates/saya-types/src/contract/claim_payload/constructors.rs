//! Validating constructors for [`ClaimPayload`](super::ClaimPayload).
//!
//! Every public constructor bounds and control-char-checks its free-text and
//! name fields, so a payload can never carry oversized or control-character
//! text into a rendered context block. Each returns a typed
//! [`ContractError`](crate::contract::error::ContractError) on rejection.

use super::ClaimPayload;
use crate::contract::claim::{MAX_REFERENCED_COLUMNS, validate_text};
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::contract::error::ContractError;
use crate::contract::identity::{DatabaseObjectRef, validate_name};

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
