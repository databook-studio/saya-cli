async fn remember_confirmed_default_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _fingerprint: &saya_types::SchemaFingerprint,
    column: &str,
) {
    put_item(
        store,
        obj,
        ClaimPayload::default_time_column(column, None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
}

async fn remember_candidate_default_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _fingerprint: &saya_types::SchemaFingerprint,
    column: &str,
) {
    put_item(
        store,
        obj,
        ClaimPayload::default_time_column(column, None).unwrap(),
        KnowledgeState::Pending,
    )
    .await;
}

/// The orders table, for fingerprinting a confirmed claim as `current`.
fn orders_table() -> Table {
    Table {
        name: "orders".into(),
        primary_key: vec![],
        foreign_keys: vec![],
        columns: vec![
            Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            Column {
                name: "created_at".into(),
                data_type: "timestamp".into(),
                nullable: false,
            },
        ],
    }
}

/// Seeds a knowledge item into the D-3 `knowledge_items` table the recall
/// path reads. The slot and `SchemaBinding` are derived from the payload the
/// way the ingest path derives them, so validity classifies the item the same
/// way a harness-learned one would. `state` picks `Active` (a confirmed fact)
/// or `Pending` (a candidate). The fingerprint is the unobserved sentinel the
/// headless/learning write path uses.
async fn put_item(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
    state: KnowledgeState,
) {
    use saya_store::{KnowledgeItemRequest, KnowledgeItemStore};
    use saya_types::SchemaBinding;
    let slot = slot_for(&payload);
    let binding = SchemaBinding::derive(&slot, &payload).expect("slot/payload agree");
    let request = KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value: payload,
        source: if state == KnowledgeState::Active {
            ClaimOrigin::UserExplicit
        } else {
            ClaimOrigin::AssistantInferred
        },
        state,
        schema_binding_json: serde_json::to_string(&binding).unwrap(),
        fingerprint: crate::commands::unobserved_fingerprint(),
    };
    store.put_knowledge_item(request).await.unwrap();
}

/// Like [`put_item`] but lets the caller override the serialised `SchemaBinding`
/// and the `fingerprint_version`, for the validity tests that need a binding
/// the ingest path would not derive (a non-current version, a hand-shaped
/// binding).
async fn put_item_with_binding(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
    state: KnowledgeState,
    schema_binding_json: String,
    fingerprint_version: u32,
) {
    use saya_store::{KnowledgeItemRequest, KnowledgeItemStore};
    use saya_types::SchemaFingerprint;
    let slot = slot_for(&payload);
    let request = KnowledgeItemRequest {
        object: object.clone(),
        slot,
        value: payload,
        source: if state == KnowledgeState::Active {
            ClaimOrigin::UserExplicit
        } else {
            ClaimOrigin::AssistantInferred
        },
        state,
        schema_binding_json,
        fingerprint: SchemaFingerprint::from_parts(fingerprint_version, &"0".repeat(64)).unwrap(),
    };
    store.put_knowledge_item(request).await.unwrap();
}

/// The slot a payload files under, mirroring the ingest path's pairing.
fn slot_for(payload: &ClaimPayload) -> KnowledgeSlot {
    match payload {
        ClaimPayload::TableDescription { .. } => KnowledgeSlot::TableDescription,
        ClaimPayload::TableAlias { .. } => KnowledgeSlot::TableAlias,
        ClaimPayload::TableGrain { .. } => KnowledgeSlot::TableGrain,
        ClaimPayload::DefaultTimeColumn { .. } => KnowledgeSlot::TableDefaultTime,
        ClaimPayload::ColumnDescription { column, .. } => KnowledgeSlot::ColumnDescription {
            column: column.clone(),
        },
        ClaimPayload::ColumnRole { column, .. } => KnowledgeSlot::ColumnRole {
            column: column.clone(),
        },
        ClaimPayload::JoinRule { .. } => KnowledgeSlot::RelationJoinRule,
        ClaimPayload::MetricDefinition { .. } => KnowledgeSlot::MetricDefinition,
        ClaimPayload::TableUserNote { .. } => KnowledgeSlot::TableUserNote,
        _ => panic!("no slot for payload {:?}", payload),
    }
}

/// The acceptance scenario: store the cached live schema matching the claim,
/// then store a confirmed `default_time_column` item under that schema. Recall
/// reads `knowledge_items`; the item's `SchemaBinding` (Column{created_at, Time})
/// is satisfied by the cached `orders` table, so it classifies `current`.
async fn seed_orders_with_created_at(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> (DatabaseObjectRef, saya_types::SchemaFingerprint) {
    let obj = object(identity, "orders");
    let tree = orders_schema(identity);
    store
        .upsert_schema(identity.as_str(), &tree.1)
        .await
        .unwrap();
    let fp = live_fingerprint(&orders_table());
    put_item(
        store,
        &obj,
        ClaimPayload::default_time_column("created_at", None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let _ = fp;
    (obj, fp)
}

// ---------------------------------------------------------------------------
// Test 1: the acceptance scenario
// ---------------------------------------------------------------------------
