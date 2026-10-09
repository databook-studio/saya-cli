#[test]
fn relationships_map_from_ref_and_source() {
    let import = parse_default(&manifest_file("dbt-rels", V12));

    // ref('customers') resolves to the selected customers node, whose object
    // carries the manifest's database, schema, and (alias-less) name.
    let ref_item = relationship_local(&import, "customer_id").expect("ref relationship maps");
    assert_eq!(ref_item.object.name, "public_orders");
    let PortablePayload::Relationship {
        target,
        local_columns,
        target_columns,
        cardinality,
        ..
    } = &ref_item.payload
    else {
        panic!("expected a relationship payload");
    };
    assert_eq!(
        (
            target.catalog.as_deref(),
            target.schema.as_deref(),
            target.name.as_str()
        ),
        (Some("analytics"), Some("core"), "customers")
    );
    assert_eq!(target.kind, DatabaseObjectKind::Table);
    assert_eq!(local_columns, &["customer_id".to_string()]);
    assert_eq!(target_columns, &["id".to_string()]);
    assert_eq!(*cardinality, Cardinality::ManyToOne);
    assert_eq!(
        ref_item.origin_note.as_deref(),
        Some("dbt 1.8.6 test.jaffle.relationships_orders_customer_id")
    );

    // source('jaffle', 'events') resolves to the selected source, named by its
    // identifier, and the target column may ride in kwargs.arguments.
    let source_item = relationship_local(&import, "event_id").expect("source relationship maps");
    let PortablePayload::Relationship {
        target,
        local_columns,
        target_columns,
        ..
    } = &source_item.payload
    else {
        panic!("expected a relationship payload");
    };
    assert_eq!(
        (
            target.catalog.as_deref(),
            target.schema.as_deref(),
            target.name.as_str()
        ),
        (Some("raw"), Some("jaffle"), "raw_events")
    );
    assert_eq!(local_columns, &["event_id".to_string()]);
    assert_eq!(target_columns, &["event_key".to_string()]);

    // The test attached to another selected node resolves ref('orders') to
    // orders' alias-bearing object.
    let hidden_item = relationship_local(&import, "ref_code").expect("hidden relationship maps");
    assert_eq!(hidden_item.object.name, "hidden");
    let PortablePayload::Relationship { target, .. } = &hidden_item.payload else {
        panic!("expected a relationship payload");
    };
    assert_eq!(target.name, "public_orders");

    // Non-relationships tests (not_null) never map, and the fixture's full
    // mapping is: 3 orders items, 1 customers, 2 events, 3 relationships.
    assert!(
        !serde_json::to_string(&import.items)
            .unwrap()
            .contains("not_null_orders_order_id")
    );
    assert_eq!(import.items.len(), 9);
}

#[test]
fn oversize_unique_id_notes_are_skipped() {
    let long_uid = format!("model.jaffle.{}", "m".repeat(300));
    let manifest = serde_json::json!({
        "metadata": {
            "dbt_schema_version": "https://schemas.getdbt.com/dbt/manifest/v12.json",
            "dbt_version": "1.8.6"
        },
        "nodes": {
            long_uid.clone(): {
                "resource_type": "model",
                "name": "wide",
                "database": "d",
                "schema": "s",
                "description": "A description that cannot carry its provenance."
            }
        }
    });
    let import = parse_default(&manifest_file("dbt-note", &manifest.to_string()));
    assert!(import.items.is_empty());
    assert_eq!(import.skipped, vec![(long_uid, "origin_note_oversize")]);
}

#[test]
fn missing_dbt_version_note_falls_back_to_unknown() {
    let manifest = r#"{"metadata":{"dbt_schema_version":"https://schemas.getdbt.com/dbt/manifest/v12.json"},"nodes":{"model.jaffle.orders":{"resource_type":"model","name":"orders","database":"d","schema":"s","description":"Desc."}}}"#;
    let import = parse_default(&manifest_file("dbt-noversion", manifest));
    assert_eq!(
        import.items[0].origin_note.as_deref(),
        Some("dbt unknown model.jaffle.orders")
    );
}
