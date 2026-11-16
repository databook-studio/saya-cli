/// The mapping's credential pre-check mirrors the store's batch admission: an
/// item whose serialised payload the redactor would rewrite is refused in the
/// mapping — reported, never offered to the batch — while the clean items
/// beside it stay planned.
#[test]
fn a_credential_shaped_claim_is_unavailable_not_planned() {
    let schema = tree(&[
        ("analytics.public", "orders", &[("id", "bigint")]),
        ("analytics.public", "customers", &[("id", "bigint")]),
    ]);
    let items = vec![
        item(
            "orders",
            &ClaimPayload::table_description("One row per confirmed order.").unwrap(),
        ),
        item("orders", &ClaimPayload::table_alias("customers").unwrap()),
        item(
            "customers",
            &ClaimPayload::table_description("The ingest box runs with password=hunter2.").unwrap(),
        ),
    ];
    let mapped = map_items(&items, &schema, &identity());
    assert_eq!(mapped.planned.len(), 2, "{:?}", mapped.unavailable);
    assert_eq!(mapped.unavailable.len(), 1, "{:?}", mapped.unavailable);
    assert_eq!(mapped.unavailable[0].label, "analytics.public.customers");
    assert_eq!(mapped.unavailable[0].reason, "credential-shaped text");
}

#[tokio::test]
async fn export_and_import_round_trip_through_the_file() {
    let root = std::env::temp_dir().join(format!(
        "saya-portable-roundtrip-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let profile = identity();
    let object = DatabaseObjectRef::new(
        profile.clone(),
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("customers").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload.clone(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(1, &"0".repeat(64)).unwrap(),
        })
        .await
        .unwrap();

    let path = root.join("context.json");
    let outcome = export(&store, &profile, &path, false).await.unwrap();
    assert_eq!(outcome.total, 1);
    assert_eq!(outcome.kinds, vec![("table_alias".to_string(), 1)]);

    // The file parses, and the imported claim files under a fresh profile's
    // identity as a Pending team-file row.
    let document = read_context(&path).unwrap();
    assert_eq!(document.format, CONTEXT_FORMAT);
    assert_eq!(document.version, CONTEXT_FORMAT_VERSION);
    let fresh = ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap();
    let outcome = import_document(
        &store,
        &fresh,
        &document.items,
        &SchemaTree::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(outcome.total(), 1);
    assert_eq!(
        outcome.unavailable.len(),
        1,
        "the empty tree resolves nothing"
    );

    // An oversize read is refused before parsing; the cap is exact.
    let oversize = root.join("oversize.json");
    std::fs::File::create(&oversize)
        .unwrap()
        .set_len(MAX_DOCUMENT_BYTES as u64 + 1)
        .unwrap();
    assert!(matches!(
        read_context(&oversize),
        Err(PortableError::Document(ContextError::Oversize(_)))
    ));

    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
