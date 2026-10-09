use super::*;
use crate::cli::ClaimKindArg as K;
use saya_types::ClaimPayload;

/// Sentinel that must never appear in any rendered [`ArgError`] message.
const SENTINEL: &str = "SENTINELVALUE";

#[test]
fn parse_kind_canonical_and_aliases() {
    // Kebab (headless clap) and snake_case aliases map to the same variant.
    assert_eq!(parse_kind("alias"), Some(K::Alias));
    assert_eq!(parse_kind("table-alias"), Some(K::Alias));
    assert_eq!(parse_kind("time-column"), Some(K::TimeColumn));
    assert_eq!(parse_kind("time_column"), Some(K::TimeColumn));
    assert_eq!(parse_kind("column-role"), Some(K::ColumnRole));
    assert_eq!(parse_kind("column_role"), Some(K::ColumnRole));
    assert_eq!(parse_kind("description"), Some(K::Description));
    assert_eq!(parse_kind("grain"), Some(K::Grain));
    assert_eq!(parse_kind("column-description"), Some(K::ColumnDescription));
}

#[test]
fn parse_kind_trims_and_lowercases() {
    assert_eq!(parse_kind("  Alias  "), Some(K::Alias));
    assert_eq!(parse_kind("ALIAS"), Some(K::Alias));
}

#[test]
fn parse_kind_unknown_is_none() {
    assert_eq!(parse_kind("not-a-kind"), None);
    assert_eq!(parse_kind(""), None);
}

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
    let p = build_payload(K::Description, "orders fact table", None, None).unwrap();
    assert!(matches!(p, ClaimPayload::TableDescription { .. }));
}

#[test]
fn build_payload_alias() {
    let p = build_payload(K::Alias, "orders", None, None).unwrap();
    assert!(matches!(p, ClaimPayload::TableAlias { .. }));
}

#[test]
fn build_payload_grain() {
    let p = build_payload(K::Grain, "one row per order", None, None).unwrap();
    assert!(matches!(p, ClaimPayload::TableGrain { .. }));
}

/// A `--reason` on a directive kind is carried onto the payload (the point
/// of `claim-reasons`: the explicit-remember path can state *why*). A reason
/// on a non-directive kind (description/alias) is ignored — only the
/// directive constructors accept one.
#[test]
fn build_payload_carries_reason_on_directive_kinds() {
    let grain = build_payload(
        K::Grain,
        "one row per order",
        None,
        Some("orders ship separately"),
    )
    .unwrap();
    assert!(matches!(
        grain,
        ClaimPayload::TableGrain { reason: Some(r), .. } if r == "orders ship separately"
    ));
    let time = build_payload(
        K::TimeColumn,
        "return_date",
        None,
        Some("a rental only counts once it comes back"),
    )
    .unwrap();
    assert!(matches!(
        time,
        ClaimPayload::DefaultTimeColumn { reason: Some(r), .. }
        if r == "a rental only counts once it comes back"
    ));
    let role = build_payload(K::ColumnRole, "measure", Some("amount"), Some("money paid")).unwrap();
    assert!(matches!(
        role,
        ClaimPayload::ColumnRole { reason: Some(r), .. } if r == "money paid"
    ));
    // A reason on a non-directive kind is not forwarded (the description
    // constructor takes none); it does not error, it is simply not carried.
    let desc = build_payload(K::Description, "a table", None, Some("ignored")).unwrap();
    assert!(matches!(desc, ClaimPayload::TableDescription { .. }));
}

#[test]
fn build_payload_column_description() {
    let p = build_payload(K::ColumnDescription, "order total", Some("amount"), None).unwrap();
    assert!(matches!(p, ClaimPayload::ColumnDescription { .. }));
}

#[test]
fn build_payload_column_role() {
    let p = build_payload(K::ColumnRole, "measure", Some("amount"), None).unwrap();
    assert!(matches!(p, ClaimPayload::ColumnRole { .. }));
}

#[test]
fn column_kinds_require_column() {
    assert_eq!(
        build_payload(K::ColumnDescription, "x", None, None).unwrap_err(),
        ArgError::ColumnRequired
    );
    assert_eq!(
        build_payload(K::ColumnRole, "measure", None, None).unwrap_err(),
        ArgError::ColumnRequired
    );
}

#[test]
fn table_kinds_reject_column() {
    for kind in [K::Description, K::Alias, K::Grain, K::TimeColumn] {
        assert_eq!(
            build_payload(kind, "x", Some("amount"), None).unwrap_err(),
            ArgError::ColumnNotApplicable
        );
    }
}

#[test]
fn unknown_column_role() {
    assert_eq!(
        build_payload(K::ColumnRole, "not-a-role", Some("amount"), None).unwrap_err(),
        ArgError::UnknownColumnRole
    );
}

#[test]
fn invalid_value_control_character() {
    let err = build_payload(
        K::Description,
        &format!("hello{SENTINEL}\nworld"),
        None,
        None,
    )
    .unwrap_err();
    assert_eq!(err, ArgError::InvalidValue);
}

#[test]
fn invalid_value_too_long() {
    let long = format!("{SENTINEL}{}", "x".repeat(2000));
    let err = build_payload(K::Description, &long, None, None).unwrap_err();
    assert_eq!(err, ArgError::InvalidValue);
}

#[test]
fn arg_error_display_never_echoes_value() {
    // Each reachable ArgError is built from input containing SENTINEL and
    // must omit it from its rendered message.
    let invalid = build_payload(K::Description, &format!("{SENTINEL}\n"), None, None).unwrap_err();
    assert_eq!(invalid, ArgError::InvalidValue);
    assert!(!format!("{invalid}").contains(SENTINEL));

    let not_applicable = build_payload(K::Description, "ok", Some(SENTINEL), None).unwrap_err();
    assert_eq!(not_applicable, ArgError::ColumnNotApplicable);
    assert!(!format!("{not_applicable}").contains(SENTINEL));

    let required = build_payload(K::ColumnRole, SENTINEL, None, None).unwrap_err();
    assert_eq!(required, ArgError::ColumnRequired);
    assert!(!format!("{required}").contains(SENTINEL));

    let unknown = build_payload(K::ColumnRole, SENTINEL, Some("amount"), None).unwrap_err();
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
