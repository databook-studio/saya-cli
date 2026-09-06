use super::*;

#[test]
fn parse_remember_table_kind_value() {
    let spec = parse_remember("a.b.c alias customers").unwrap();
    assert_eq!(spec.table, "a.b.c");
    assert_eq!(spec.kind, ClaimKindArg::Alias);
    assert_eq!(spec.value, "customers");
    assert_eq!(spec.column, None);
    assert_eq!(spec.reason, None);
}

#[test]
fn parse_remember_directive_kind_carries_a_because_reason() {
    // The motivating case: a time-column claim with the reason a user would
    // state in one breath.
    let spec = parse_remember(
        "pagila.public.rental time-column return_date because a rental only counts once it comes back",
    )
    .unwrap();
    assert_eq!(spec.kind, ClaimKindArg::TimeColumn);
    assert_eq!(spec.value, "return_date");
    assert_eq!(
        spec.reason.as_deref(),
        Some("a rental only counts once it comes back")
    );
    // A grain with a reason.
    let grain =
        parse_remember("a.b.c grain one row per order because orders ship separately").unwrap();
    assert_eq!(grain.value, "one row per order");
    assert_eq!(grain.reason.as_deref(), Some("orders ship separately"));
    // A column-role with a reason.
    let role =
        parse_remember("a.b.c column-role amount measure because money the customer paid").unwrap();
    assert_eq!(role.value, "measure");
    assert_eq!(role.reason.as_deref(), Some("money the customer paid"));
}

#[test]
fn parse_remember_because_is_not_split_for_non_directive_kinds() {
    // A description legitimately contains "because"; the directive
    // constructors are the only ones that accept a reason, so for a
    // description the whole tail stays the value and no reason is split.
    let spec = parse_remember("a.b.c description returns because the warehouse closes").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::Description);
    assert_eq!(spec.value, "returns because the warehouse closes");
    assert_eq!(spec.reason, None);
}

#[test]
fn parse_remember_because_substring_is_not_a_split() {
    // "because" as a substring of a larger word is not a delimiter.
    let spec = parse_remember("a.b.c time-column created_at probecause_marker").unwrap();
    assert_eq!(spec.value, "created_at probecause_marker");
    assert_eq!(spec.reason, None);
    // A trailing `because` with no reason clause is not a split either.
    let bare = parse_remember("a.b.c time-column created_at because").unwrap();
    assert_eq!(bare.value, "created_at because");
    assert_eq!(bare.reason, None);
}

#[test]
fn parse_remember_directive_without_because_has_no_reason() {
    let spec = parse_remember("a.b.c time-column created_at").unwrap();
    assert_eq!(spec.value, "created_at");
    assert_eq!(spec.reason, None);
}

#[test]
fn parse_remember_value_keeps_spaces() {
    let spec = parse_remember("a.b.c description orders fact table").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::Description);
    assert_eq!(spec.value, "orders fact table");
    assert_eq!(spec.column, None);
}

#[test]
fn parse_remember_column_kind_takes_column_then_value() {
    let spec = parse_remember("a.b.c column-description amount order total").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::ColumnDescription);
    assert_eq!(spec.column.as_deref(), Some("amount"));
    assert_eq!(spec.value, "order total");
}

#[test]
fn parse_remember_column_role() {
    let spec = parse_remember("a.b.c column-role amount measure").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::ColumnRole);
    assert_eq!(spec.column.as_deref(), Some("amount"));
    assert_eq!(spec.value, "measure");
}

#[test]
fn parse_remember_time_column_is_table_scoped() {
    let spec = parse_remember("a.b.c time-column created_at").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::TimeColumn);
    assert_eq!(spec.value, "created_at");
    assert_eq!(spec.column, None);
}

#[test]
fn parse_remember_kind_aliases_accepted() {
    // snake_case alias maps to the same variant as the canonical kebab form.
    let spec = parse_remember("a.b.c column_description amount note").unwrap();
    assert_eq!(spec.kind, ClaimKindArg::ColumnDescription);
}

#[test]
fn parse_remember_unknown_kind_is_usage_error_without_echo() {
    let bad = parse_remember("a.b.c not-a-kind value").unwrap_err();
    assert!(!bad.0.contains("not-a-kind"));
    assert!(!bad.0.contains("a.b.c"));
    assert!(bad.0.contains("kind"));
}

#[test]
fn parse_remember_too_few_args_is_usage_error() {
    assert!(parse_remember("").is_err());
    assert!(parse_remember("a.b.c").is_err());
    assert!(parse_remember("a.b.c alias").is_err());
    // column kind with no column.
    assert!(parse_remember("a.b.c column-description").is_err());
    assert!(parse_remember("a.b.c column-description amount").is_err());
}
