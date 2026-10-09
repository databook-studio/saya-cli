fn orders_schema(tables: &[&str]) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|name| Table {
                        name: (*name).into(),
                        columns: vec![Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    })
                    .collect(),
            }],
        }],
    }
}

/// A schema shaped like the dbt v12 fixture's models and source.
fn dbt_schema() -> SchemaTree {
    SchemaTree {
        databases: vec![
            Database {
                name: "analytics".into(),
                schemas: vec![Schema {
                    name: "core".into(),
                    tables: ["public_orders", "customers", "hidden"]
                        .iter()
                        .map(|name| Table {
                            name: (*name).into(),
                            columns: vec![Column {
                                name: "customer_id".into(),
                                data_type: "integer".into(),
                                nullable: true,
                            }],
                            primary_key: vec![],
                            foreign_keys: vec![],
                        })
                        .collect(),
                }],
            },
            Database {
                name: "raw".into(),
                schemas: vec![Schema {
                    name: "jaffle".into(),
                    tables: vec![Table {
                        name: "raw_events".into(),
                        columns: vec![Column {
                            name: "event_key".into(),
                            data_type: "text".into(),
                            nullable: true,
                        }],
                        primary_key: vec![],
                        foreign_keys: vec![],
                    }],
                }],
            },
        ],
    }
}

/// Remembers a claim through the real `remember` adapter, so the exported
/// Active claim is the row the ordinary write path produces.
async fn remember_active(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
) {
    let command = ContractsCommand::Remember {
        table: "analytics.public.orders".into(),
        kind,
        value: value.into(),
        column: column.map(str::to_string),
        reason: None,
        profile: None,
    };
    let (code, out, err) = run(command, runtime, store, RenderFormat::Text).await;
    assert_eq!(code, 0, "remember failed: {out}{err}");
}

/// Seeds a non-Active row directly, to prove the export excludes it.
async fn seed_other_state(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    state: KnowledgeState,
) {
    let profile = ProfileIdentity::parse(&identity_for(runtime, "local")).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("ghost-alias").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object,
            slot,
            value: payload,
            source: ClaimOrigin::AssistantInferred,
            state,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
}

/// A stored item for `object`, for state assertions. `qualified` is
/// `catalog.schema.object` as the mapping resolved it.
async fn stored_items(
    store: &SqliteStateStore,
    runtime: &RuntimeConfig,
    profile: Option<&str>,
    qualified: &str,
) -> Vec<saya_store::KnowledgeItem> {
    let identity = identity_for(runtime, profile.unwrap_or("local"));
    let profile_id = ProfileIdentity::parse(&identity).unwrap();
    let mut parts = qualified.split('.');
    let (catalog, schema, object) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(c), Some(s), Some(o), None) => (c, s, o),
        _ => panic!("qualified must be catalog.schema.object: {qualified}"),
    };
    let object = DatabaseObjectRef::new(
        profile_id,
        catalog,
        schema,
        object,
        DatabaseObjectKind::Table,
    )
    .unwrap();
    store.knowledge_for_object(&object).await.unwrap()
}

// ---------------------------------------------------------------------------
// context-document builders
// ---------------------------------------------------------------------------

fn context_item(catalog: &str, schema: &str, name: &str, payload: &ClaimPayload) -> ContextItem {
    ContextItem {
        object: PortableObject {
            catalog: Some(catalog.into()),
            schema: Some(schema.into()),
            name: name.into(),
            kind: DatabaseObjectKind::Table,
        },
        payload: PortablePayload::from_claim(payload).unwrap(),
        origin_note: None,
    }
}

fn context_document(items: Vec<ContextItem>) -> String {
    ContextDocumentV1 {
        format: CONTEXT_FORMAT.into(),
        version: CONTEXT_FORMAT_VERSION,
        exported_unix_ms: 0,
        items,
    }
    .to_json_pretty()
    .unwrap()
}

fn write_context_file(path: &Path, json: &str) {
    fs::write(path, json).unwrap();
}

// ---------------------------------------------------------------------------
// export: Active only, kinds counted, overwrite/symlink/dir refusals
// ---------------------------------------------------------------------------
