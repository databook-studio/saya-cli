//! `ClaimPayload` constructor validation: text bounds and control
//! characters, and relationship column-list pairing, emptiness, and count
//! bounds.

use super::*;

#[test]
fn test_9_claim_payload_rejects_empty_text() {
    assert!(ClaimPayload::table_description("").is_err());
    assert!(ClaimPayload::table_grain("", None).is_err());
    assert!(ClaimPayload::column_description("col", "").is_err());
}

#[test]
fn test_9_claim_payload_rejects_long_text() {
    let long = "x".repeat(1025);
    assert!(ClaimPayload::table_description(&long).is_err());
    assert!(ClaimPayload::table_grain(&long, None).is_err());
    assert!(ClaimPayload::column_description("col", &long).is_err());
}

#[test]
fn test_9_claim_payload_rejects_text_with_newline() {
    assert!(ClaimPayload::table_description("hello\nworld").is_err());
    assert!(ClaimPayload::table_grain("hello\nworld", None).is_err());
    assert!(ClaimPayload::column_description("col", "hello\nworld").is_err());
}

#[test]
fn test_9_relationship_rejects_mismatched_column_lists() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    let result = ClaimPayload::relationship(
        target,
        vec!["a".into(), "b".into()],
        vec!["x".into()],
        Cardinality::OneToOne,
    );
    assert_eq!(result.unwrap_err(), ContractError::ColumnCountMismatch);
}

#[test]
fn test_9_relationship_rejects_empty_column_list() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    assert!(
        ClaimPayload::relationship(target, vec![], vec!["x".into()], Cardinality::OneToOne)
            .is_err()
    );
}

#[test]
fn test_9_relationship_rejects_33_columns() {
    let profile = valid_profile();
    let target = DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap();
    let cols: Vec<String> = (0..=MAX_REFERENCED_COLUMNS)
        .map(|i| format!("c{i}"))
        .collect();
    let result = ClaimPayload::relationship(target, cols.clone(), cols, Cardinality::OneToMany);
    assert_eq!(result.unwrap_err(), ContractError::TooManyColumns);
}
