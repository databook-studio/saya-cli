use super::*;

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
fn serde_validates_payloads_but_accepts_blanked_tombstones() {
    let oversized = serde_json::json!({
        "kind": "table_description",
        "text": "x".repeat(crate::MAX_TEXT_CHARS + 1),
    });
    assert!(serde_json::from_value::<ClaimPayload>(oversized).is_err());

    let control = serde_json::json!({
        "kind": "table_alias",
        "alias": "orders\nsecret",
    });
    assert!(serde_json::from_value::<ClaimPayload>(control).is_err());

    let blanked = ClaimPayload::table_alias("orders").unwrap().blanked();
    let json = serde_json::to_value(&blanked).unwrap();
    assert_eq!(
        serde_json::from_value::<ClaimPayload>(json).unwrap(),
        blanked
    );
}
