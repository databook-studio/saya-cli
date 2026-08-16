use saya_store::{
    ContractStore, KnowledgeItemRequest, KnowledgeItemStore, KnowledgeStoreError, ProposeClaim,
    ProposeOutcome, SqliteStateStore, StoreError,
};
use saya_types::{
    ClaimOrigin, ClaimPayload, ClaimStatus, ColumnRole, DatabaseObjectKind, DatabaseObjectRef,
    KnowledgeSlot, KnowledgeState, ProfileIdentity, SchemaFingerprint,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-knowledge-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile(hex_char: char) -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", hex_char.to_string().repeat(64))).unwrap()
}

fn object(profile: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

/// A binding whose version is not the current `FINGERPRINT_VERSION`, to prove
/// the version round-trips rather than being laundered through the build's
/// constant on read.
fn binding(version: u32) -> (SchemaFingerprint, String) {
    (
        SchemaFingerprint::from_parts(version, &"f".repeat(64)).unwrap(),
        r#"{"columns":["user_id"]}"#.to_owned(),
    )
}

fn request(
    object: &DatabaseObjectRef,
    slot: KnowledgeSlot,
    value: ClaimPayload,
    source: ClaimOrigin,
    state: KnowledgeState,
    version: u32,
) -> KnowledgeItemRequest {
    let (fingerprint, schema_binding_json) = binding(version);
    KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value,
        source,
        state,
        schema_binding_json,
        fingerprint,
    }
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

/// Spec test 1: insert and read back one item; every field round-trips,
/// including the binding and its version.
#[tokio::test]
async fn insert_and_read_back_round_trips_every_field() {
    let root = temp_root("t1");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let (fingerprint, schema_binding_json) = binding(7);
    let value = ClaimPayload::table_grain("one row per shipped order").unwrap();
    store
        .put_knowledge_item(KnowledgeItemRequest {
            object: obj.clone(),
            slot: KnowledgeSlot::TableGrain,
            value: value.clone(),
            source: ClaimOrigin::UserExplicit,
            state: KnowledgeState::Active,
            schema_binding_json: schema_binding_json.clone(),
            fingerprint,
        })
        .await
        .unwrap();

    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.object, obj);
    assert_eq!(item.slot, KnowledgeSlot::TableGrain);
    assert!(item.cardinality_single, "table.grain is single-valued");
    assert_eq!(item.value, value);
    assert_eq!(item.source, ClaimOrigin::UserExplicit);
    assert_eq!(item.state, KnowledgeState::Active);
    // The binding and its version round-trip verbatim — not re-derived.
    assert_eq!(item.schema_binding_json, schema_binding_json);
    assert_eq!(item.fingerprint_version, 7);
    assert!(item.created_unix_ms > 0);
    assert_eq!(item.updated_unix_ms, item.created_unix_ms);
    assert!(item.id.starts_with("ki-"));
    let _ = fs::remove_dir_all(root);
}

/// Spec test 2: writing a second value to a single-valued slot replaces — one
/// row after, not two. Asserted at the database level, not just through the
/// API. The partial unique index is the contract; prove it directly too.
#[tokio::test]
async fn second_value_to_single_slot_replaces_at_the_database() {
    let root = temp_root("t2");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");

    let first = ClaimPayload::table_grain("one row per order").unwrap();
    let second = ClaimPayload::table_grain("one row per shipment").unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            first,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            second.clone(),
            ClaimOrigin::AssistantInferred,
            KnowledgeState::Pending,
            2,
        ))
        .await
        .unwrap();

    // API read: one item, holding the second value.
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 1, "replace must not grow a single-valued slot");
    assert_eq!(items[0].value, second);
    assert_eq!(items[0].source, ClaimOrigin::AssistantInferred);
    assert_eq!(items[0].state, KnowledgeState::Pending);
    assert_eq!(items[0].fingerprint_version, 2);

    // Database level: exactly one row, and it carries the second value.
    let pool = read_pool(&db).await;
    let (count, value_json, source): (i64, String, String) = sqlx::query_as(
        "SELECT COUNT(*), value_json, source FROM knowledge_items WHERE profile_id=? AND catalog=? AND schema=? AND object=? AND object_kind=? AND slot='table.grain'",
    )
    .bind(obj.profile().as_str())
    .bind(obj.catalog())
    .bind(obj.schema())
    .bind(obj.object())
    .bind(obj.kind().as_str())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let restored: ClaimPayload = serde_json::from_str(&value_json).unwrap();
    assert_eq!(restored, second);
    assert_eq!(source, "assistant_inferred");
    pool.close().await;

    // The unique index itself — independent of the id scheme — refuses two
    // single-valued rows for the same object+slot. Two rows with distinct ids
    // and distinct values must collide on the partial index.
    let pool = read_pool(&db).await;
    sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-direct-a', ?, ?, ?, ?, ?, 'column:created_at.role', 'single', '{}', 'user_explicit', 'active', '{}', 1, 1, 1)")
        .bind(obj.profile().as_str())
        .bind(obj.catalog())
        .bind(obj.schema())
        .bind(obj.object())
        .bind(obj.kind().as_str())
        .execute(&pool)
        .await
        .unwrap();
    let collision = sqlx::query("INSERT INTO knowledge_items(id, profile_id, catalog, schema, object, object_kind, slot, cardinality, value_json, source, state, schema_binding_json, fingerprint_version, created_unix_ms, updated_unix_ms) VALUES ('ki-direct-b', ?, ?, ?, ?, ?, 'column:created_at.role', 'single', '{}', 'user_explicit', 'active', '{}', 1, 1, 1)")
        .bind(obj.profile().as_str())
        .bind(obj.catalog())
        .bind(obj.schema())
        .bind(obj.object())
        .bind(obj.kind().as_str())
        .execute(&pool)
        .await;
    assert!(
        collision.is_err(),
        "knowledge_items is missing its single-valued UNIQUE index"
    );
    pool.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Spec test 3: a multi-valued slot accepts more than one value, up to its
/// bound, and refuses past it with a typed error.
#[tokio::test]
async fn multi_valued_slot_accepts_up_to_its_bound() {
    let root = temp_root("t3");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let slot = KnowledgeSlot::TableAlias;
    for i in 0..saya_types::MAX_MULTI_SLOT_VALUES {
        let alias = ClaimPayload::table_alias(format!("alias{i}")).unwrap();
        store
            .put_knowledge_item(request(
                &obj,
                slot.clone(),
                alias,
                ClaimOrigin::UserExplicit,
                KnowledgeState::Active,
                1,
            ))
            .await
            .unwrap();
    }
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(
        items.len(),
        saya_types::MAX_MULTI_SLOT_VALUES,
        "multi-valued slot accepts up to its bound"
    );
    // One past the bound is a typed refusal, and it stores nothing.
    let too_many = ClaimPayload::table_alias("one too many").unwrap();
    let error = store
        .put_knowledge_item(request(
            &obj,
            slot,
            too_many,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::BoundExceeded);
    assert_eq!(
        store.knowledge_for_object(&obj).await.unwrap().len(),
        saya_types::MAX_MULTI_SLOT_VALUES,
        "a refused append must store nothing"
    );
    // Re-filing an existing value is an idempotent update, not a duplicate and
    // not a bound refusal.
    let again = ClaimPayload::table_alias("alias0").unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableAlias,
            again,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    assert_eq!(
        store.knowledge_for_object(&obj).await.unwrap().len(),
        saya_types::MAX_MULTI_SLOT_VALUES
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 4: two column-scoped slots for different columns coexist.
#[tokio::test]
async fn column_scoped_slots_for_different_columns_coexist() {
    let root = temp_root("t4");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::ColumnRole {
                column: "created_at".into(),
            },
            ClaimPayload::column_role("created_at", ColumnRole::Timestamp).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::ColumnRole {
                column: "updated_at".into(),
            },
            ClaimPayload::column_role("updated_at", ColumnRole::Timestamp).unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    let items = store.knowledge_for_object(&obj).await.unwrap();
    assert_eq!(items.len(), 2, "two column-scoped slots coexist");
    let columns: Vec<String> = items
        .iter()
        .filter_map(|item| item.slot.column().map(String::from))
        .collect();
    assert!(columns.contains(&"created_at".to_string()));
    assert!(columns.contains(&"updated_at".to_string()));
    let _ = fs::remove_dir_all(root);
}

/// Spec test 5: a read for profile A never returns profile B's rows.
#[tokio::test]
async fn read_for_one_profile_never_returns_anothers_rows() {
    let root = temp_root("t5");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj_a = object(&profile('a'), "orders");
    let obj_b = object(&profile('b'), "orders");
    store
        .put_knowledge_item(request(
            &obj_a,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile a grain").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_b,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("profile b grain").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();

    let a_items = store.knowledge_for_profile(&profile('a')).await.unwrap();
    assert_eq!(a_items.len(), 1);
    assert_eq!(a_items[0].object.profile(), &profile('a'));
    let b_items = store.knowledge_for_profile(&profile('b')).await.unwrap();
    assert_eq!(b_items.len(), 1);
    assert_eq!(b_items[0].object.profile(), &profile('b'));
    // The same qualified name under profile A never leaks into profile B.
    assert!(
        a_items
            .iter()
            .all(|item| item.object.profile() == &profile('a'))
    );
    assert!(
        b_items
            .iter()
            .all(|item| item.object.profile() == &profile('b'))
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec test 6: one query returns everything known for a profile.
#[tokio::test]
async fn one_query_returns_everything_known_for_a_profile() {
    let root = temp_root("t6");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj_orders = object(&profile('a'), "orders");
    let obj_events = object(&profile('a'), "events");
    // Several slots across two objects — a description (multi), a grain
    // (single), and two column roles.
    store
        .put_knowledge_item(request(
            &obj_orders,
            KnowledgeSlot::TableDescription,
            ClaimPayload::table_description("the orders table").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_orders,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per order").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    store
        .put_knowledge_item(request(
            &obj_events,
            KnowledgeSlot::ColumnRole {
                column: "occurred_at".into(),
            },
            ClaimPayload::column_role("occurred_at", ColumnRole::Timestamp).unwrap(),
            ClaimOrigin::TeamFile,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap();
    let items = store.knowledge_for_profile(&profile('a')).await.unwrap();
    // Three rows across two objects, from one query.
    assert_eq!(items.len(), 3);
    let objects: Vec<&str> = items.iter().map(|i| i.object.object()).collect();
    assert!(objects.contains(&"orders"));
    assert!(objects.contains(&"events"));
    let _ = fs::remove_dir_all(root);
}

/// Spec test 7: the existing `contract_*` tables and their behaviour are
/// unaffected. A claim round-trips through the unchanged store path on a
/// database migrated to the new version, and the knowledge table coexists.
#[tokio::test]
async fn existing_contract_tables_are_unaffected() {
    let root = temp_root("t7");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    // The unchanged claim path still works on a v5 database.
    let payload = ClaimPayload::table_description("the orders table").unwrap();
    let request = ProposeClaim {
        object: obj.clone(),
        fingerprint: SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap(),
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.payload, Some(payload));

    // The knowledge table coexists alongside the contract tables, and a
    // knowledge write does not disturb the claim.
    store
        .put_knowledge_item(request_knowledge(
            &obj,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_grain("one row per order").unwrap(),
        ))
        .await
        .unwrap();
    assert!(store.knowledge_for_object(&obj).await.unwrap().len() == 1);
    assert!(
        store
            .list_claims(&obj, &[ClaimStatus::Confirmed])
            .await
            .unwrap()
            .len()
            == 1,
        "the claim survives the knowledge write"
    );
    let _ = fs::remove_dir_all(root);
}

fn request_knowledge(
    object: &DatabaseObjectRef,
    slot: KnowledgeSlot,
    value: ClaimPayload,
) -> KnowledgeItemRequest {
    request(
        object,
        slot,
        value,
        ClaimOrigin::UserExplicit,
        KnowledgeState::Active,
        1,
    )
}

/// A mismatched payload under a slot is a typed `CardinalityMismatch`, storing
/// nothing. Not one of the seven spec tests, but the typed-refusal surface is
/// part of the deliverable.
#[tokio::test]
async fn mismatched_payload_under_a_slot_is_refused() {
    let root = temp_root("mismatch");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            ClaimPayload::table_alias("not a grain").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::CardinalityMismatch);
    assert!(
        store.knowledge_for_object(&obj).await.unwrap().is_empty(),
        "a refused write must store nothing"
    );
    // Confirm the typed error stays payload-free.
    let rendered = error.to_string();
    assert!(!rendered.contains("not a grain"));
    let _ = fs::remove_dir_all(root);
}

/// A value that structurally resembles a secret is refused, not stored — the
/// same discipline as the claim payload.
#[tokio::test]
async fn secret_shaped_value_is_refused_not_stored() {
    let root = temp_root("secret");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            // A description whose text contains a credential header shape.
            ClaimPayload::table_grain("x-api-key: SUPERSECRETVALUE").unwrap(),
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await;
    assert!(error.is_err(), "a secret-shaped value must be refused");
    let err = error.unwrap_err();
    assert_eq!(err, KnowledgeStoreError::Store(StoreError::Invalid));
    assert!(
        store.knowledge_for_object(&obj).await.unwrap().is_empty(),
        "a refused value must not be stored"
    );
    let _ = fs::remove_dir_all(root);
}

/// A `relationship` payload matches no slot, so it is refused as a
/// cardinality mismatch — the value vocabulary has no slot to file it under.
#[tokio::test]
async fn relationship_payload_matches_no_slot() {
    let root = temp_root("rel");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let obj = object(&profile('a'), "orders");
    let target = object(&profile('a'), "users");
    let rel = ClaimPayload::relationship(
        target,
        vec!["user_id".into()],
        vec!["id".into()],
        saya_types::Cardinality::ManyToOne,
    )
    .unwrap();
    let error = store
        .put_knowledge_item(request(
            &obj,
            KnowledgeSlot::TableGrain,
            rel,
            ClaimOrigin::UserExplicit,
            KnowledgeState::Active,
            1,
        ))
        .await
        .unwrap_err();
    assert_eq!(error, KnowledgeStoreError::CardinalityMismatch);
    let _ = fs::remove_dir_all(root);
}
