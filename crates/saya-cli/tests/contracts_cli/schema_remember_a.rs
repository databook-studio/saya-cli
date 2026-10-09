#[tokio::test]
async fn remember_against_cached_schema_reads_current_not_needs_review() {
    let root = temp_root("remember_current_not_needs_review");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    // Cache the schema — what `connection schema local --refresh` would write.
    // `store_at` cached an empty default for its own scope; overwrite it for
    // this profile's identity with the real orders schema.
    let identity = identity_for(&runtime, "local");
    store
        .upsert_schema(&identity, &orders_schema())
        .await
        .unwrap();

    // Remember AFTER the refresh. Before the fix this stored the all-zeros
    // sentinel; now it must store the real digest of the cached orders table.
    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::Alias,
        value: "customers".into(),
        column: None,
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    // The stored item carries the current fingerprint version and a `Table`
    // binding derived from the alias slot. This is the load-bearing assertion
    // under the binding model: `item_validity_for` classifies via the binding +
    // version (not a stored digest — `knowledge_items` persists only the version),
    // so a current version + a `Table` binding the cached `orders` table satisfies
    // is what makes the claim read `current` instead of `needs_review`.
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "expected one item: {items:?}");
    assert_eq!(
        items[0].fingerprint_version, FINGERPRINT_VERSION,
        "remember must store the item under the current fingerprint version"
    );
    let binding: SchemaBinding =
        serde_json::from_str(&items[0].schema_binding_json).expect("binding deserializes");
    assert!(
        matches!(binding, SchemaBinding::Table),
        "a table-alias stores a Table binding, got {binding:?}"
    );
    assert_eq!(items[0].state, KnowledgeState::Active);

    // `show` classifies the claim against the same cached schema and must read
    // `current` — the user-facing symptom.
    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(
        out.contains("[current]"),
        "remember-then-show must read current against the cached schema: {out}"
    );
    assert!(
        !out.contains("needs_review"),
        "a claim remembered right after a refresh must not read needs_review: {out}"
    );

    let _ = fs::remove_dir_all(root);
}

// A column-level claim remembered against a cached schema stores a
// `SchemaBinding::Column` naming the column and its semantic requirement, so
// the claim reads `current` against a cache that has the column. The binding
// model replaces the legacy typed column snapshot: a fact depends on a
// column *existing* (or being temporal/numeric for the role-bearing kinds), not
// on the connector's exact type string — so a harmless widening no longer
// reads `needs_review`. The guarantee kept: a column claim right after a
// refresh reads `current`, not `needs_review`.
#[tokio::test]
async fn remember_column_claim_against_cached_schema_snapshots_real_type() {
    let root = temp_root("remember_column_snapshot_real_type");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;
    let identity = identity_for(&runtime, "local");
    // orders table with the `id` column the claim references.
    let schema = SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![Table {
                    name: "orders".into(),
                    primary_key: vec![],
                    foreign_keys: vec![],
                    columns: vec![Column {
                        name: "id".into(),
                        data_type: "bigint".into(),
                        nullable: false,
                    }],
                }],
            }],
        }],
    };
    store.upsert_schema(&identity, &schema).await.unwrap();

    let remember = ContractsCommand::Remember {
        table: qualified().into(),
        kind: ClaimKindArg::ColumnRole,
        value: "identifier".into(),
        column: Some("id".into()),
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(remember, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember stderr: {err}");
    assert!(out.contains("remembered"), "out: {out}");

    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let items = store.knowledge_for_object(&object).await.unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    // The binding names the column and its semantic requirement. An
    // `identifier` role requires the column to exist (`Exists`), so a cache
    // that has `id` reads `current`.
    let binding: SchemaBinding =
        serde_json::from_str(&items[0].schema_binding_json).expect("binding deserializes");
    assert_eq!(
        binding,
        SchemaBinding::Column {
            column: "id".into(),
            requirement: ColumnRequirement::Exists,
        },
        "a column-role claim stores a Column binding naming the column: {binding:?}"
    );
    assert_eq!(items[0].state, KnowledgeState::Active);

    let show = ContractsCommand::Show {
        table: qualified().into(),
        profile: None,
    };
    let (code, out, err) = run(show, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(code, 0, "show stderr: {err}");
    assert!(out.contains("[current]"), "must read current: {out}");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 14. Symptom A regression: `remember` against an object the cached schema
//     does NOT contain must refuse at remember time — naming the object and
//     suggesting `connection schema --refresh` — and must store nothing.
//     Before the fix a typo'd catalog/schema/object stored happily as
//     confirmed and only a later refresh marked it stale.
// ---------------------------------------------------------------------------
