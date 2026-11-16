#[tokio::test]
async fn queue_never_leaks_the_opaque_profile_identity() {
    let root = temp_root("queue_identity");
    let (runtime, _c, _n) = runtime_at(&root);
    let store = store_at(&root).await;

    let _id = seed_candidate(&store, &runtime, "orders").await;
    let identity = identity_for(&runtime, "local");
    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    for format in [RenderFormat::Text, RenderFormat::Json, RenderFormat::Ndjson] {
        let (code, out, err) = run(queue.clone(), &runtime, &store, format).await;
        assert_eq!(code, 0, "queue {format:?} stderr: {err}");
        for captured in [out.as_str(), err.as_str()] {
            assert!(
                !captured.contains(&identity),
                "opaque identity leaked into queue {format:?}: {captured}"
            );
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_unopenable_store_exits_nonzero_like_list() {
    let root = temp_root("queue_unopenable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);
    let (runtime, _c, _n) = runtime_at(&root);

    let queue = ContractsCommand::Queue {
        profile: None,
        limit: None,
    };
    let (qcode, qout, qerr) = run(queue, &runtime, &store, RenderFormat::Text).await;
    assert_ne!(
        qcode, 0,
        "queue must exit non-zero on an unopenable store: {qout}{qerr}"
    );
    assert!(
        !format!("{qout}{qerr}").is_empty(),
        "queue must emit a diagnostic on an unopenable store"
    );

    // The failure matches `list` — "no candidates" and "could not read" stay
    // distinguishable the same way "no contracts" and "unreadable" do.
    let list = ContractsCommand::List { profile: None };
    let (lcode, lout, lerr) = run(list, &runtime, &store, RenderFormat::Text).await;
    assert_eq!(qcode, lcode, "queue exit diverged from list");
    assert_eq!(qout, lout, "queue stdout diverged from list");
    assert_eq!(qerr, lerr, "queue stderr diverged from list");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// 12. `contracts list` / `show` report schema state from the cached schema,
//     not a constant `live_schema_unavailable`. Regression for a bug found by
//     exercising the built binary: both commands passed `schemas: &[]` (list)
//     and `unobserved_fingerprint()` (show) and never loaded the cached schema,
//     so validity could only ever compute LiveSchemaUnavailable — even right
//     after a `connection schema --refresh` that populated the cache. Every
//     unit test built the schemas explicitly, so no test caught it.
//
//     The claim is seeded with the *real* fingerprint of the cached table (not
//     the unobserved sentinel `remember` stores), so a matching cache reads
//     `current`. When no schema is cached, `live_schema_unavailable` stays the
//     honest answer — that path is asserted too, so the fix cannot regress to
//     fabricating `current` from an empty cache.
// ---------------------------------------------------------------------------
fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        columns: vec![Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn orders_schema() -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "analytics".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: vec![orders_table()],
            }],
        }],
    }
}

/// Seeds a confirmed (Active) knowledge item for `orders` whose
/// `SchemaBinding` the cached `orders` table satisfies, so a matching cache
/// classifies it `current`. The item is written under the current fingerprint
/// version with a `Table` binding (a table-alias depends only on the table
/// existing), which is the shape a `remember`-written alias takes.
async fn seed_current_claim(store: &SqliteStateStore, runtime: &RuntimeConfig) {
    let identity = identity_for(runtime, "local");
    let profile = ProfileIdentity::parse(&identity).unwrap();
    let object = DatabaseObjectRef::new(
        profile,
        "analytics",
        "public",
        "orders",
        DatabaseObjectKind::Table,
    )
    .unwrap();
    let payload = ClaimPayload::table_alias("orders").unwrap();
    let slot = KnowledgeSlot::TableAlias;
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: object.clone(),
            slot,
            value: payload,
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: serde_json::to_string(&binding).unwrap(),
            // Current version; the digest is not persisted on the row (only the
            // version is), so the all-zero sentinel stands in — `item_validity_for`
            // classifies via the binding + version, not a stored digest.
            fingerprint: SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
                .unwrap(),
        })
        .await
        .unwrap();
}
