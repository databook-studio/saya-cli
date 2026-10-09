//! The claim's displayed value and directive reason.
//!
//! Single sources shared beyond the prompt body: the recall receipt and the
//! propose tool read them so no consumer can name a value the prompt did not.

use saya_types::ClaimPayload;

/// The reason a directive claim carries, when it has one. Only the directive
/// variants (`DefaultTimeColumn`, `TableGrain`, `ColumnRole`) carry a reason;
/// every other variant (and a directive with no reason) returns `None`. This
/// is the single source the prompt body reads for the reason, so a future
/// consumer cannot render a reason the prompt did not.
pub(crate) fn claim_reason(payload: &ClaimPayload) -> Option<&str> {
    match payload {
        ClaimPayload::DefaultTimeColumn { reason, .. } => reason.as_deref(),
        ClaimPayload::TableGrain { reason, .. } => reason.as_deref(),
        ClaimPayload::ColumnRole { reason, .. } => reason.as_deref(),
        ClaimPayload::JoinRule { reason, .. } => reason.as_deref(),
        ClaimPayload::MetricDefinition { reason, .. } => reason.as_deref(),
        _ => None,
    }
}

/// The short rendered value for a claim, plus the column name when the claim is
/// column-scoped. This is the single source of truth for what a claim "shows"
/// as a value: the prompt body ([`super::claim_line`]) and the P1a recall
/// receipt both read it here, so the receipt can never name a value the prompt
/// did not; the propose tool reuses it so a `KnowledgeProposed` event names the
/// same value. `value` is `""` only for a payload shape this module does not
/// render (a future variant); the `kind` still identifies it.
pub(crate) fn claim_value(payload: &ClaimPayload) -> (Option<String>, String) {
    match payload {
        ClaimPayload::TableDescription { text, .. } => (None, text.clone()),
        ClaimPayload::TableAlias { alias, .. } => (None, alias.clone()),
        ClaimPayload::TableGrain { description, .. } => (None, description.clone()),
        ClaimPayload::ColumnDescription { column, text, .. } => {
            (Some(column.clone()), text.clone())
        }
        ClaimPayload::ColumnRole { column, role, .. } => {
            (Some(column.clone()), role.as_str().to_string())
        }
        ClaimPayload::DefaultTimeColumn { column, .. } => (None, column.clone()),
        // The join condition is the fact; the target table it names lives in
        // the payload and is not a column on the local object.
        ClaimPayload::JoinRule { condition, .. } => (None, condition.clone()),
        // A metric shows as `name = definition` so the line names which metric
        // the formula belongs to, not just the formula in isolation.
        ClaimPayload::MetricDefinition {
            name, definition, ..
        } => (None, format!("{name} = {definition}")),
        ClaimPayload::TableUserNote { text, .. } => (None, text.clone()),
        // `Relationship` is not exposed on the CLI yet; a future
        // variant is handled here too. No value leaks for an unknown shape.
        _ => (None, String::new()),
    }
}
