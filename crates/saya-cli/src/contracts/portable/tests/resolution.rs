#[test]
fn resolution_is_case_insensitive_and_ambiguous_when_several_match() {
    let schema = tree(&[("analytics.public", "orders", &[("id", "bigint")])]);
    let items = vec![item(
        "ORDERS",
        &ClaimPayload::table_description("One row per order.").unwrap(),
    )];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.unavailable.len(), 0);
    assert_eq!(mapped.planned[0].label, "analytics.public.orders");

    // Two same-spelled tables in different schemas under one catalog: with the
    // schema named the resolution is exact, without it the spelling never
    // picks and the item is unavailable.
    let second_schema = Schema {
        name: "private".into(),
        tables: schema.databases[0].schemas[0].tables.clone(),
    };
    let doubled = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![schema.databases[0].schemas[0].clone(), second_schema],
        }],
    };
    let ambiguous = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: None,
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(
            &ClaimPayload::table_description("One row per order.").unwrap(),
        )
        .unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&ambiguous, &doubled, &identity());
    assert_eq!(mapped.planned.len(), 0);
    assert_eq!(
        mapped.unavailable[0].reason,
        "the name matches several objects in this profile's schema"
    );

    // The schema named, the same document resolves again.
    let named = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(&ClaimPayload::table_alias("customers").unwrap())
            .unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&named, &doubled, &identity());
    assert_eq!(mapped.planned.len(), 1);
}

#[test]
fn a_relationship_files_as_its_keyed_join_rule() {
    let schema = tree(&[
        ("analytics.public", "orders", &[("customer_id", "bigint")]),
        ("analytics.public", "customers", &[("id", "bigint")]),
    ]);
    // Build the portable relationship through the validating constructors: the
    // synthetic profile stands in for the identity the constructor requires.
    let profile = identity();
    let target = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "customers",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let claim = ClaimPayload::relationship(
        target,
        vec!["customer_id".into()],
        vec!["id".into()],
        saya_types::Cardinality::ManyToOne,
    )
    .unwrap();
    let items = vec![ContextItem {
        object: PortableObject {
            catalog: Some("analytics".into()),
            schema: Some("public".into()),
            name: "orders".into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(&claim).unwrap(),
        origin_note: None,
    }];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.unavailable.len(), 0, "{:?}", mapped.unavailable);
    let planned = &mapped.planned[0];
    assert_eq!(planned.slot, KnowledgeSlot::RelationJoinRule.as_str());
    let ClaimPayload::JoinRule {
        target,
        local_columns,
        target_columns,
        condition,
        ..
    } = &planned.item.value
    else {
        panic!("expected a filed join rule");
    };
    assert_eq!(target, "analytics.public.customers");
    assert_eq!(local_columns, &["customer_id".to_string()]);
    assert_eq!(target_columns, &["id".to_string()]);
    assert_eq!(condition, "customer_id = id");
    // The binding rides the local keys, so the fact invalidates when the key
    // column disappears.
    assert!(planned.item.schema_binding_json.contains("customer_id"));
}
