//! Parsing and validation for the headless contracts CLI — slice 2b-2a.
//!
//! This module only parses and validates arguments. It must not touch the store,
//! the schema, or the registry; dispatch and rendering are later slices. Every
//! fallible constructor in `saya_types` is used through `build_payload`, and any
//! `ContractError` it returns is mapped to [`ArgError::InvalidValue`] *without*
//! echoing the offending value — the value is untrusted input the store refuses
//! to persist, and echoing it into a terminal message would undo that refusal.

use thiserror::Error;

use crate::cli::ClaimKindArg;
use saya_types::{ClaimPayload, ColumnRole, KnowledgeSlot};

/// A three-part qualified name `catalog.schema.object`. Never guessed: a one-
/// or two-part name binds the claim to an object the user did not name, and the
/// binding is the whole point of a contract.
#[derive(Debug)]
pub(crate) struct QualifiedName {
    pub catalog: String,
    pub schema: String,
    pub object: String,
}

pub(crate) fn parse_qualified(input: &str) -> Result<QualifiedName, ArgError> {
    let parts: Vec<&str> = input.split('.').map(str::trim).collect();
    if parts.len() != 3 {
        return Err(ArgError::MalformedQualifiedName);
    }
    let [catalog, schema, object] = parts.as_slice() else {
        return Err(ArgError::MalformedQualifiedName);
    };
    if catalog.is_empty() || schema.is_empty() || object.is_empty() {
        return Err(ArgError::MalformedQualifiedName);
    }
    Ok(QualifiedName {
        catalog: catalog.to_string(),
        schema: schema.to_string(),
        object: object.to_string(),
    })
}

/// Maps a kind word to [`ClaimKindArg`]. Accepts clap's canonical kebab-case
/// `--kind` values (the headless form) and snake_case aliases for typing
/// friendliness; both map to the same variant so the agent tool, the slash
/// adapter, and the CLI all agree on what `time-column` means. This is the one
/// kind-word parser in the crate — the headless clap `--kind` derives its
/// vocabulary from `ClaimKindArg`'s `ValueEnum`, and the slash adapter and the
/// `contract_propose` agent tool both call this so no second vocabulary exists.
pub(crate) fn parse_kind(word: &str) -> Option<ClaimKindArg> {
    match word.trim().to_ascii_lowercase().as_str() {
        "description" | "table-description" => Some(ClaimKindArg::Description),
        "alias" | "table-alias" => Some(ClaimKindArg::Alias),
        "grain" | "table-grain" => Some(ClaimKindArg::Grain),
        "time-column" | "time_column" => Some(ClaimKindArg::TimeColumn),
        "column-description" | "column_description" => Some(ClaimKindArg::ColumnDescription),
        "column-role" | "column_role" => Some(ClaimKindArg::ColumnRole),
        _ => None,
    }
}

pub(crate) fn build_payload(
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
    reason: Option<&str>,
) -> Result<ClaimPayload, ArgError> {
    use ClaimKindArg as K;
    // A `ContractError` from a fallible `saya_types` constructor is mapped to
    // `InvalidValue` *without* carrying the offending value — see `ArgError`.
    // We use `.map_err(|_| ArgError::InvalidValue)` rather than `?` so the error
    // never becomes a `ContractError`-carrying variant, and the value stays out.
    //
    // `reason` is forwarded to the directive constructors (`Grain`, `TimeColumn`,
    // `ColumnRole`) only — a reason on a description or alias is not applicable,
    // and the non-directive constructors do not accept one. An empty/whitespace
    // reason collapses to `None` inside the constructor, so a `--reason ""` is
    // the same as no `--reason`.
    match kind {
        K::Description => {
            reject_column(column)?;
            ClaimPayload::table_description(value).map_err(|_| ArgError::InvalidValue)
        }
        K::Alias => {
            reject_column(column)?;
            ClaimPayload::table_alias(value).map_err(|_| ArgError::InvalidValue)
        }
        K::Grain => {
            reject_column(column)?;
            ClaimPayload::table_grain(value, reason).map_err(|_| ArgError::InvalidValue)
        }
        K::TimeColumn => {
            reject_column(column)?;
            ClaimPayload::default_time_column(value, reason).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnDescription => {
            let column = require_column(column)?;
            ClaimPayload::column_description(column, value).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnRole => {
            let column = require_column(column)?;
            let role = ColumnRole::parse(value).ok_or(ArgError::UnknownColumnRole)?;
            ClaimPayload::column_role(column, role, reason).map_err(|_| ArgError::InvalidValue)
        }
    }
}

/// The [`KnowledgeSlot`] a `build_payload` payload files under — the one
/// pairing the ingest path and the `remember` write path both use, so a
/// remembered fact lands on the same row `show`/`queue`/recall read (no split
/// brain). Returns `None` for a payload `build_payload` cannot produce (e.g. a
/// `Relationship`), so a caller fails closed rather than guessing a slot.
/// `build_payload` and `slot_for_payload` are the only place the kind→slot
/// pairing lives; the in-crate `contracts/tests.rs` and the integration suites
/// reach for it through here instead of carrying a second copy that could
/// drift.
pub(crate) fn slot_for_payload(payload: &ClaimPayload) -> Option<KnowledgeSlot> {
    match payload {
        ClaimPayload::TableDescription { .. } => Some(KnowledgeSlot::TableDescription),
        ClaimPayload::TableAlias { .. } => Some(KnowledgeSlot::TableAlias),
        ClaimPayload::TableGrain { .. } => Some(KnowledgeSlot::TableGrain),
        ClaimPayload::DefaultTimeColumn { .. } => Some(KnowledgeSlot::TableDefaultTime),
        ClaimPayload::ColumnDescription { column, .. } => Some(KnowledgeSlot::ColumnDescription {
            column: column.clone(),
        }),
        ClaimPayload::ColumnRole { column, .. } => Some(KnowledgeSlot::ColumnRole {
            column: column.clone(),
        }),
        ClaimPayload::JoinRule { .. } => Some(KnowledgeSlot::RelationJoinRule),
        ClaimPayload::MetricDefinition { .. } => Some(KnowledgeSlot::MetricDefinition),
        // `Relationship` is not slot-bound on the `remember` path;
        // `build_payload` never produces it, so a caller that reaches one has a
        // payload it cannot file.
        _ => None,
    }
}

#[derive(Debug)]
pub(crate) enum ReviewDecision {
    Confirm,
    Reject,
}

pub(crate) fn review_decision(confirm: bool, reject: bool) -> Result<ReviewDecision, ArgError> {
    match (confirm, reject) {
        (true, false) => Ok(ReviewDecision::Confirm),
        (false, true) => Ok(ReviewDecision::Reject),
        _ => Err(ArgError::AmbiguousReviewDecision),
    }
}

fn require_column(column: Option<&str>) -> Result<&str, ArgError> {
    column
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or(ArgError::ColumnRequired)
}

fn reject_column(column: Option<&str>) -> Result<(), ArgError> {
    if let Some(c) = column
        && !c.trim().is_empty()
    {
        return Err(ArgError::ColumnNotApplicable);
    }
    Ok(())
}

/// Errors from contracts argument parsing. Payload-free: a bad `--kind` is
/// caught by clap before this code runs, so no variant carries the offending
/// value. Echoing untrusted input into a terminal message is exactly what the
/// store refuses to persist it for.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub(crate) enum ArgError {
    #[error("qualified name must be exactly three dot-separated parts: catalog.schema.object")]
    MalformedQualifiedName,
    #[error("this claim kind does not accept a column")]
    ColumnNotApplicable,
    #[error("this claim kind requires a column")]
    ColumnRequired,
    #[error("unknown column role")]
    UnknownColumnRole,
    #[error("claim value is invalid")]
    InvalidValue,
    #[error("choose exactly one of --confirm or --reject")]
    AmbiguousReviewDecision,
}

#[cfg(test)]
#[path = "args/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "args/property_tests.rs"]
mod property_tests;
