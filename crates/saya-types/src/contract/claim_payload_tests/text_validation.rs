use super::*;

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
fn table_user_note_is_validated_and_round_trips() {
    let note = ClaimPayload::table_user_note("Keep refunds separate from sales.").unwrap();
    let json = serde_json::to_string(&note).unwrap();
    let decoded: ClaimPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, note);
    assert!(ClaimPayload::table_user_note("").is_err());
    assert!(ClaimPayload::table_user_note("line\nbreak").is_err());
    assert!(ClaimPayload::table_user_note("x".repeat(MAX_TEXT_CHARS + 1)).is_err());
    assert_eq!(
        note.blanked(),
        ClaimPayload::TableUserNote {
            text: String::new()
        }
    );
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
