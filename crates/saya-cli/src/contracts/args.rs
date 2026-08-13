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
use saya_types::{ClaimPayload, ColumnRole};

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

pub(crate) fn build_payload(
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
) -> Result<ClaimPayload, ArgError> {
    use ClaimKindArg as K;
    // A `ContractError` from a fallible `saya_types` constructor is mapped to
    // `InvalidValue` *without* carrying the offending value — see `ArgError`.
    // We use `.map_err(|_| ArgError::InvalidValue)` rather than `?` so the error
    // never becomes a `ContractError`-carrying variant, and the value stays out.
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
            ClaimPayload::table_grain(value).map_err(|_| ArgError::InvalidValue)
        }
        K::TimeColumn => {
            reject_column(column)?;
            ClaimPayload::default_time_column(value).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnDescription => {
            let column = require_column(column)?;
            ClaimPayload::column_description(column, value).map_err(|_| ArgError::InvalidValue)
        }
        K::ColumnRole => {
            let column = require_column(column)?;
            let role = ColumnRole::parse(value).ok_or(ArgError::UnknownColumnRole)?;
            ClaimPayload::column_role(column, role).map_err(|_| ArgError::InvalidValue)
        }
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
mod tests {
    use super::*;
    use crate::cli::ClaimKindArg as K;
    use saya_types::ClaimPayload;

    /// Sentinel that must never appear in any rendered [`ArgError`] message.
    const SENTINEL: &str = "SENTINELVALUE";

    #[test]
    fn parse_qualified_accepts_three_parts() {
        let q = parse_qualified("analytics.public.orders").unwrap();
        assert_eq!(q.catalog, "analytics");
        assert_eq!(q.schema, "public");
        assert_eq!(q.object, "orders");
    }

    #[test]
    fn parse_qualified_rejects_wrong_arity() {
        assert!(parse_qualified("orders").is_err());
        assert!(parse_qualified("public.orders").is_err());
        assert!(parse_qualified("a.b.c.d").is_err());
    }

    #[test]
    fn parse_qualified_rejects_empty_part() {
        assert!(parse_qualified("a..c").is_err());
        assert!(parse_qualified("  .b.c").is_err());
    }

    #[test]
    fn parse_qualified_trims_whitespace_around_parts() {
        let spaced = parse_qualified(" analytics . public . orders ").unwrap();
        let plain = parse_qualified("analytics.public.orders").unwrap();
        assert_eq!(spaced.catalog, plain.catalog);
        assert_eq!(spaced.schema, plain.schema);
        assert_eq!(spaced.object, plain.object);
    }

    #[test]
    fn build_payload_description() {
        let p = build_payload(K::Description, "orders fact table", None).unwrap();
        assert!(matches!(p, ClaimPayload::TableDescription { .. }));
    }

    #[test]
    fn build_payload_alias() {
        let p = build_payload(K::Alias, "orders", None).unwrap();
        assert!(matches!(p, ClaimPayload::TableAlias { .. }));
    }

    #[test]
    fn build_payload_grain() {
        let p = build_payload(K::Grain, "one row per order", None).unwrap();
        assert!(matches!(p, ClaimPayload::TableGrain { .. }));
    }

    #[test]
    fn build_payload_time_column() {
        let p = build_payload(K::TimeColumn, "created_at", None).unwrap();
        assert!(matches!(p, ClaimPayload::DefaultTimeColumn { .. }));
    }

    #[test]
    fn build_payload_column_description() {
        let p = build_payload(K::ColumnDescription, "order total", Some("amount")).unwrap();
        assert!(matches!(p, ClaimPayload::ColumnDescription { .. }));
    }

    #[test]
    fn build_payload_column_role() {
        let p = build_payload(K::ColumnRole, "measure", Some("amount")).unwrap();
        assert!(matches!(p, ClaimPayload::ColumnRole { .. }));
    }

    #[test]
    fn column_kinds_require_column() {
        assert_eq!(
            build_payload(K::ColumnDescription, "x", None).unwrap_err(),
            ArgError::ColumnRequired
        );
        assert_eq!(
            build_payload(K::ColumnRole, "measure", None).unwrap_err(),
            ArgError::ColumnRequired
        );
    }

    #[test]
    fn table_kinds_reject_column() {
        for kind in [K::Description, K::Alias, K::Grain, K::TimeColumn] {
            assert_eq!(
                build_payload(kind, "x", Some("amount")).unwrap_err(),
                ArgError::ColumnNotApplicable
            );
        }
    }

    #[test]
    fn unknown_column_role() {
        assert_eq!(
            build_payload(K::ColumnRole, "not-a-role", Some("amount")).unwrap_err(),
            ArgError::UnknownColumnRole
        );
    }

    #[test]
    fn invalid_value_control_character() {
        let err =
            build_payload(K::Description, &format!("hello{SENTINEL}\nworld"), None).unwrap_err();
        assert_eq!(err, ArgError::InvalidValue);
    }

    #[test]
    fn invalid_value_too_long() {
        let long = format!("{SENTINEL}{}", "x".repeat(2000));
        let err = build_payload(K::Description, &long, None).unwrap_err();
        assert_eq!(err, ArgError::InvalidValue);
    }

    #[test]
    fn arg_error_display_never_echoes_value() {
        // Each reachable ArgError is built from input containing SENTINEL and
        // must omit it from its rendered message.
        let invalid = build_payload(K::Description, &format!("{SENTINEL}\n"), None).unwrap_err();
        assert_eq!(invalid, ArgError::InvalidValue);
        assert!(!format!("{invalid}").contains(SENTINEL));

        let not_applicable = build_payload(K::Description, "ok", Some(SENTINEL)).unwrap_err();
        assert_eq!(not_applicable, ArgError::ColumnNotApplicable);
        assert!(!format!("{not_applicable}").contains(SENTINEL));

        let required = build_payload(K::ColumnRole, SENTINEL, None).unwrap_err();
        assert_eq!(required, ArgError::ColumnRequired);
        assert!(!format!("{required}").contains(SENTINEL));

        let unknown = build_payload(K::ColumnRole, SENTINEL, Some("amount")).unwrap_err();
        assert_eq!(unknown, ArgError::UnknownColumnRole);
        assert!(!format!("{unknown}").contains(SENTINEL));

        let malformed = parse_qualified(SENTINEL).unwrap_err();
        assert_eq!(malformed, ArgError::MalformedQualifiedName);
        assert!(!format!("{malformed}").contains(SENTINEL));

        let ambiguous = review_decision(true, true).unwrap_err();
        assert_eq!(ambiguous, ArgError::AmbiguousReviewDecision);
        assert!(!format!("{ambiguous}").contains(SENTINEL));
    }

    #[test]
    fn review_decision_confirm_only() {
        assert!(matches!(
            review_decision(true, false).unwrap(),
            ReviewDecision::Confirm
        ));
    }

    #[test]
    fn review_decision_reject_only() {
        assert!(matches!(
            review_decision(false, true).unwrap(),
            ReviewDecision::Reject
        ));
    }

    #[test]
    fn review_decision_ambiguous() {
        assert_eq!(
            review_decision(false, false).unwrap_err(),
            ArgError::AmbiguousReviewDecision
        );
        assert_eq!(
            review_decision(true, true).unwrap_err(),
            ArgError::AmbiguousReviewDecision
        );
    }
}
