fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contract-ops-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_a() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn profile_b() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "b".repeat(64))).unwrap()
}

/// A fixed "now" for the model-path freshness bound. Far from any boundary so
/// a cache observed at 0 (the test default) is well within the 24h bound and
/// the age gate never fires — these tests exercise recall/validity, not the
/// bound (which has its own unit tests in `availability` and `validity`).
const FRESH_NOW: i64 = 1_000_000;

/// Wraps a tree as a fresh `Available` schema (observed at 0, well within the
/// bound against `FRESH_NOW`) so a test's inline schema classifies normally.
fn avail(tree: SchemaTree) -> SchemaAvailability {
    SchemaAvailability::available(tree, 0)
}

/// Builds the `(profile, availability)` list recall/review_queue/reconcile take,
/// mapping the `(profile, tree)` pairs tests naturally build. Each tree is
/// fresh (`avail`), so the model-path age gate does not fire.
fn schemas_for(
    profile: &ProfileIdentity,
    trees: &[SchemaTree],
) -> Vec<(ProfileIdentity, SchemaAvailability)> {
    trees
        .iter()
        .map(|tree| (profile.clone(), avail(tree.clone())))
        .collect()
}

fn object_ref(profile: &ProfileIdentity, name: &str) -> DatabaseObjectRef {
    DatabaseObjectRef::new(
        profile.clone(),
        "catalog",
        "public",
        name,
        DatabaseObjectKind::Table,
    )
    .unwrap()
}

fn table(cols: &[(&str, &str, bool)]) -> Table {
    Table {
        name: "orders".into(),
        columns: cols
            .iter()
            .map(|(name, ty, nullable)| Column {
                name: (*name).into(),
                data_type: (*ty).into(),
                nullable: *nullable,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

fn schema_tree_for(tables: &[(&str, Table)]) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|(name, t)| Table {
                        name: (*name).into(),
                        columns: t.columns.clone(),
                        primary_key: t.primary_key.clone(),
                        foreign_keys: t.foreign_keys.clone(),
                    })
                    .collect(),
            }],
        }],
    }
}

/// Like [`schema_tree_for`] but takes owned table names, for tests that build
/// names in a loop (`format!("t{i}")`) and cannot hand the borrow to a
/// `&[(&str, Table)]` slice without leaking.
fn schema_tree_for_owned(tables: &[(String, Table)]) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "catalog".into(),
            schemas: vec![Schema {
                name: "public".into(),
                tables: tables
                    .iter()
                    .map(|(name, t)| Table {
                        name: name.clone(),
                        columns: t.columns.clone(),
                        primary_key: t.primary_key.clone(),
                        foreign_keys: t.foreign_keys.clone(),
                    })
                    .collect(),
            }],
        }],
    }
}

async fn store_at(db: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    // Touch the pool so migrations run and the schema exists.
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

/// Seeds a knowledge item into the D-3 `knowledge_items` table the recall path
/// reads. The slot is derived from the payload; the `SchemaBinding` is derived
/// from `(slot, payload)` the way the ingest path derives it, so validity
/// classifies the item the same way a harness-learned one would. `state` picks
/// `Active` (a confirmed fact) or `Pending` (a candidate). The fingerprint is
/// the unobserved sentinel the headless/learning write path uses, so an item
/// seeded without a live schema classifies against whatever cache the test
/// later installs — not against a fabricated digest that could false-match.
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
        _ => panic!("no slot for payload {:?}", payload),
    }
}

/// The `ki-…` id of the one knowledge item for `object` under `slot`, parsed as
/// a [`ClaimId`]. The store derives the id from `(object, slot)` for a
/// single-valued slot, so there is exactly one; a multi-valued slot would need
/// the value too. Used by the decision-op tests to reach the id the receipt
/// prints and `resolve_prefix` matches.
async fn item_id_for(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    slot: &KnowledgeSlot,
) -> ClaimId {
    let item = store
        .knowledge_for_object(object)
        .await
        .expect("knowledge items listed")
        .into_iter()
        .find(|i| &i.slot == slot)
        .unwrap_or_else(|| panic!("no item for slot {slot:?} on {}", object.object()));
    ClaimId::parse(&item.id).expect("ki id parses")
}

/// The persisted [`KnowledgeState`] of item `id` — the lever the decision-op
/// tests pull to assert a confirm/reject moved (or did not move) the state.
async fn item_state(store: &SqliteStateStore, id: &ClaimId) -> KnowledgeState {
    store
        .get_knowledge_item(id.as_str())
        .await
        .expect("store read")
        .expect("item present")
        .state
}

/// Seeds an `Active` `default_time_column` item on `column` for `object` — the
/// "confirmed fact whose schema then drifted" shape the confirm-revalidation
/// tests need. Under D-4 a drifted fact is `Active` (state) reading `Invalid`
/// (computed), the analog of the legacy persisted-`Stale` claim; the item's
/// `Column { column, Time }` binding reads `Valid` against a schema that has
/// `column` as a temporal type and `Invalid` against one that dropped or retyped
/// it. Returns the item's `ki-…` id.
async fn seed_active_time_item(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    column: &str,
) -> ClaimId {
    put_item(
        store,
        object,
        ClaimPayload::default_time_column(column, None).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    item_id_for(store, object, &KnowledgeSlot::TableDefaultTime).await
}
