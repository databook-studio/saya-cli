use super::*;

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
