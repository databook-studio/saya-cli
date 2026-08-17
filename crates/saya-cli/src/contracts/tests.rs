//! Integration tests for the contract-application operations layer.
//!
//! These exercise the `pub(crate)` surface that every adapter will render:
//! recall, validity, conflict detection, and the review wrappers. They are
//! in-crate (not under `tests/`) because the surface is `pub(crate)` — the
//! operations layer returns typed data for saya-cli's own adapters, not for
//! external consumers. See the spec at .claude/specs/spec-2b1-contract-operations.md.

use super::{
    ContractOpError, ContractSchemaState, RecallBounds, RecallDiagnostics, RecallMode,
    RecallRequest, RetrievalPolicy, SchemaAvailability, confirm, conflicts_for, forget, recall,
    reject, resolve_prefix, show, use_candidate_once,
};
use saya_store::{ForgetReason, KnowledgeItemStore, SchemaStore, SqliteStateStore};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, Database, DatabaseObjectKind,
    DatabaseObjectRef, KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema, SchemaTree, Table,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::contracts::{ContractConflict, RecallOutcome};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

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
        ClaimPayload::default_time_column(column).unwrap(),
        KnowledgeState::Active,
    )
    .await;
    item_id_for(store, object, &KnowledgeSlot::TableDefaultTime).await
}

fn recall_request<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    allow_database_context: bool,
    bounds: RecallBounds,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context,
        schemas,
        now_unix_ms: FRESH_NOW,
        bounds,
        // The existing recall tests model today's behaviour: confirmed only.
        // A test that needs `IncludeCandidates` builds its own request.
        recall_mode: RecallMode::Confirmed,
        // No per-claim admission on the legacy recall tests; `None` keeps the
        // mode. The `use_candidate_once` tests build their own request with a
        // real admission.
        admit_candidate: None,
        // The helper defaults to the model-facing policy so the existing tests
        // exercise the same exclusion a real prompt recall applies. A test that
        // wants the human-review path (stale kept) builds its own request.
        policy: RetrievalPolicy::ForModel,
    }
}

// ---------------------------------------------------------------------------
// Test 1: cross-profile isolation
// ---------------------------------------------------------------------------
#[tokio::test]
async fn cross_profile_alias_resolves_only_in_its_own_profile() {
    let root = temp_root("cross_profile");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let a = profile_a();
    let b = profile_b();
    let obj_a = object_ref(&a, "orders");
    let obj_b = object_ref(&b, "orders");

    put_item(
        &store,
        &obj_a,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj_b,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema_a = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&a),
            &[(a.clone(), avail(schema_a))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj_a);

    let schema_b = schema_tree_for(&[("orders", table(&[("id", "int", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&b),
            &[(b.clone(), avail(schema_b))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj_b);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: candidate claims never appear in recall
// ---------------------------------------------------------------------------
#[tokio::test]
async fn candidate_claims_never_appear_in_recall() {
    let root = temp_root("no_candidates");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // An active alias plus a pending "secret_alias" on the same object. Under
    // `Confirmed` the pending item is not admitted, so it must not reach the
    // recalled contract.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("secret_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    let claims = &outcome.contracts[0].claims;
    assert!(claims.iter().all(|c| c.status.is_recallable()));
    assert!(
        !claims.iter().any(|c| {
            matches!(
                &c.value,
                ClaimPayload::TableAlias { alias, .. } if alias == "secret_alias"
            )
        }),
        "candidate alias leaked into recall"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: bounds hold on objects and claims.
//
// The byte bound used to be asserted here too, against serialized claim payloads
// inside `recall`/`assemble`. That assertion encoded the bug the P1 fix removes:
// payload length is not the unit that reaches the request — the rendered block
// (headers, markers, conflict lines, the wrapper) is — and the bound skipped the
// first claim of every object, so it was never a bound at all. The byte bound now
// lives in the prompt-recall path, against the rendered body and the agent message
// budget; its tests are in `recall_context_tests.rs` (the layer that renders). The
// count bounds this test still pins — `max_objects` and `max_claims_per_object` —
// are the ones `recall`/`assemble` actually enforce.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn bounds_hold_on_objects_and_claims() {
    let root = temp_root("bounds");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let objs: Vec<DatabaseObjectRef> = (0..3)
        .map(|i| object_ref(&p, &format!("orders{i}")))
        .collect();
    for obj in &objs {
        put_item(
            &store,
            obj,
            ClaimPayload::table_alias(obj.object()).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    // D-3 NOTE: `table_alias` is multi-valued (max 4). The per-object bound test
    // needs more items than `max_claims_per_object` (3) on one object, so seed 4
    // aliases — the most one multi-valued slot admits — and let the cap of 3
    // truncate the tail. (The old test seeded 20 of the same slot, which D-3
    // cardinality refuses past 4.)
    let heavy = object_ref(&p, "heavy");
    for i in 0..4 {
        put_item(
            &store,
            &heavy,
            ClaimPayload::table_alias(format!("a{i}")).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }

    // max_objects = 2 over 3 matches -> exactly 2, truncated. All three orders
    // objects are present in the live schema so each reads `current` -- this
    // test is about bounds, not staleness, and a stale match would be dropped
    // by the model-facing policy before the object bound applied,
    // contaminating the count.
    let terms: Vec<String> = objs.iter().map(|o| o.object().to_string()).collect();
    let bounds = RecallBounds {
        max_objects: 2,
        max_claims_per_object: 12,
        max_bytes: 16384,
    };
    let schema = schema_tree_for(&[
        ("orders0", table(&[("id", "bigint", false)])),
        ("orders1", table(&[("id", "bigint", false)])),
        ("orders2", table(&[("id", "bigint", false)])),
    ]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &terms,
            true,
            bounds,
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 2, "max_objects not honored");
    assert!(
        outcome.contracts.iter().any(|c| c.truncated),
        "object truncation not flagged"
    );

    // max_claims_per_object = 3 on `heavy` (20 aliases) -> 3, truncated.
    let bounds2 = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 3,
        max_bytes: 16384,
    };
    let schema_h = schema_tree_for(&[("heavy", table(&[("id", "bigint", false)]))]);
    let outcome2 = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema_h.clone()))],
            &["heavy".to_string()],
            true,
            bounds2,
        ),
    )
    .await;
    assert_eq!(outcome2.contracts.len(), 1);
    assert_eq!(outcome2.contracts[0].claims.len(), 3);
    assert!(outcome2.contracts[0].truncated);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 4: ambiguous alias returns every match
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ambiguous_alias_returns_every_match() {
    let root = temp_root("ambiguous");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj1 = object_ref(&p, "table_one");
    let obj2 = object_ref(&p, "table_two");
    put_item(
        &store,
        &obj1,
        ClaimPayload::table_alias("shared").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj2,
        ClaimPayload::table_alias("shared").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[
        ("table_one", table(&[("id", "bigint", false)])),
        ("table_two", table(&[("id", "bigint", false)])),
    ]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["shared".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        2,
        "ambiguous alias should return both"
    );
    let names: Vec<&str> = outcome
        .contracts
        .iter()
        .map(|c| c.object.object())
        .collect();
    assert!(names.contains(&"table_one"));
    assert!(names.contains(&"table_two"));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5: privacy gate returns zero contracts and counts the excluded
// ---------------------------------------------------------------------------
#[tokio::test]
async fn privacy_gate_returns_zero_and_counts_excluded() {
    let root = temp_root("privacy");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            false,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "privacy gate returned contracts"
    );
    assert!(
        outcome.diagnostics.excluded_by_privacy > 0,
        "excluded_by_privacy should be non-zero when matches were suppressed"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: an unopenable store reports unavailable without erroring
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unopenable_store_reports_unavailable_without_erroring() {
    let root = temp_root("unavailable");
    // A path whose parent is a regular file cannot be created as a directory.
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let p = profile_a();
    let schema = SchemaTree::default();
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert!(outcome.diagnostics.store_unavailable);
    assert_eq!(outcome.contracts.len(), 0);
    assert_eq!(outcome.diagnostics.considered, 0);
    assert_eq!(outcome.diagnostics.selected, 0);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 8: two TableGrain claims conflict but both are returned
// ---------------------------------------------------------------------------
//
// D-3 NOTE: `table_grain` is a single-valued slot, so two confirmed grains
// cannot coexist in `knowledge_items` — the second `put` replaces the first.
// The conflict's structural precondition is gone under D-3 (see
// `knowledge_validity.rs`). This test now exercises `conflicts_for` directly
// on two constructed claims, so it still proves the detector — one
// `table_grain` conflict naming both ids — without requiring the impossible
// store state. The store-path seeding was the example; the behaviour it
// guarded is the detection.
#[test]
fn two_table_grain_claims_conflict_but_both_returned() {
    use crate::contracts::ContractClaim;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id1 = ClaimId::parse("c-grain0001").unwrap();
    let id2 = ClaimId::parse("c-grain0002").unwrap();
    let claims = vec![
        ContractClaim {
            id: id1.clone(),
            object: obj.clone(),
            value: ClaimPayload::table_grain("one row per order").unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        },
        ContractClaim {
            id: id2.clone(),
            object: obj.clone(),
            value: ClaimPayload::table_grain("one row per order line").unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        },
    ];
    let conflicts = conflicts_for(&claims);
    let grain_conflicts: Vec<&ContractConflict> = conflicts
        .iter()
        .filter(|c| c.kind == "table_grain")
        .collect();
    assert_eq!(
        grain_conflicts.len(),
        1,
        "expected one table_grain conflict"
    );
    let ids: Vec<ClaimId> = grain_conflicts[0].claim_ids.clone();
    assert!(
        ids.contains(&id1) && ids.contains(&id2),
        "conflict must name both IDs"
    );
    assert_eq!(claims.len(), 2);
}

/// SPEC REVIEW companion: two TableDescription claims do NOT conflict.
///
/// `table_description` is a multi-valued slot under D-3, so two confirmed
/// descriptions coexist in `knowledge_items`; `conflicts_for` does not treat
/// `table_description` as exclusive, so neither is flagged. Seeded through the
/// D-3 store the recall path reads.
#[tokio::test]
async fn two_table_description_claims_do_not_conflict() {
    let root = temp_root("desc_no_conflict");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("sales fact table").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("updated nightly").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    let contract = &outcome.contracts[0];
    assert!(
        contract
            .conflicts
            .iter()
            .all(|c| c.kind != "table_description"),
        "TableDescription must not be flagged as a conflict (SPEC REVIEW)"
    );
    assert_eq!(contract.claims.len(), 2);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 9: diagnostics carry no claim text
// ---------------------------------------------------------------------------
#[tokio::test]
async fn recall_diagnostics_carry_no_claim_text() {
    let root = temp_root("no_text_in_diag");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    const SENTINEL: &str = "SENTINELDIAGTEXT";
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description(format!("orders {SENTINEL} facts")).unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome: RecallOutcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    let debug = format!("{:?}", outcome.diagnostics);
    assert!(
        !debug.contains(SENTINEL),
        "diagnostics leaked claim text: {debug}"
    );
    let diags: RecallDiagnostics = outcome.diagnostics.clone();
    assert!(!format!("{diags:?}").contains(SENTINEL));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 10: a forgotten claim disappears from recall immediately
// ---------------------------------------------------------------------------
//
// Under D-3, "forgotten" is `KnowledgeState::Dismissed` on the knowledge item;
// admissibility excludes `Dismissed`, so the item no longer reaches recall.
#[tokio::test]
async fn forgotten_claim_disappears_from_recall() {
    use saya_store::KnowledgeItemStore;
    let root = temp_root("forget_disappears");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("keep").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("gone").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    // The store-assigned `ki-` ids, matched by alias value.
    let items = store
        .knowledge_for_object(&obj)
        .await
        .expect("knowledge items listed");
    let find_id = |alias: &str| {
        items
            .iter()
            .find(|i| {
                matches!(
                    &i.value,
                    ClaimPayload::TableAlias { alias: a, .. } if a == alias
                )
            })
            .map(|i| i.id.clone())
            .expect("alias item stored")
    };
    let keep = ClaimId::parse(&find_id("keep")).unwrap();
    let gone_id = find_id("gone");
    store
        .update_knowledge_item_state(&gone_id, KnowledgeState::Dismissed)
        .await
        .expect("item dismissed");

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    let contract = &outcome.contracts[0];
    let ids: Vec<ClaimId> = contract.claims.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&keep), "kept claim should remain");
    assert!(
        !ids.iter().any(|id| id.as_str() == gone_id),
        "forgotten claim should not appear"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Phase 3d: the candidate review queue
// ---------------------------------------------------------------------------
//
// `review_queue` is the opposite view from recall: recall answers "what is
// true about this question", the queue answers "what is waiting for me". It
// lists candidates only, ordered most-evidence-first then oldest then by claim
// id, so a reviewer works a stable list. See .claude/specs/spec-3d-review-queue.md.

use super::review_queue;

#[tokio::test]
async fn queue_lists_candidates_not_confirmed_or_forgotten() {
    let root = temp_root("queue_membership");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    // An Active item (confirmed) and a Dismissed one (rejected/forgotten) must
    // not appear; a Pending candidate must. The queue lists `Pending` only.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("confirmed_alias").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("cand_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let cand_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("forgotten").unwrap(),
        KnowledgeState::Dismissed,
    )
    .await;

    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        200,
    )
    .await
    .unwrap();

    let ids: Vec<ClaimId> = queued.iter().map(|q| q.claim.id.clone()).collect();
    assert!(ids.contains(&cand_id), "candidate missing from queue");
    // Every queued item is Pending — neither Active nor Dismissed leaks in.
    for id in &ids {
        assert_eq!(
            item_state(&store, id).await,
            KnowledgeState::Pending,
            "non-pending item appeared in the queue"
        );
    }
    // The queue carries one entry per Pending candidate.
    assert_eq!(queued.len(), 1);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_orders_oldest_then_slot_then_id() {
    let root = temp_root("queue_order");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let live = schema_tree_for(&[
        ("old", table(&[("id", "bigint", false)])),
        ("new", table(&[("id", "bigint", false)])),
        ("both", table(&[("id", "bigint", false)])),
    ]);

    // Oldest first: two candidates on distinct objects, created in sequence,
    // must come back oldest-first. A short sleep makes the millisecond stamps
    // differ so the primary key (created_unix_ms) decides.
    put_item(
        &store,
        &object_ref(&p, "old"),
        ClaimPayload::table_alias("old").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    put_item(
        &store,
        &object_ref(&p, "new"),
        ClaimPayload::table_alias("new").unwrap(),
        KnowledgeState::Pending,
    )
    .await;

    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let ordered: Vec<&str> = queued.iter().map(|q| q.claim.object.object()).collect();
    let old_pos = ordered
        .iter()
        .position(|o| *o == "old")
        .expect("old candidate missing");
    let new_pos = ordered
        .iter()
        .position(|o| *o == "new")
        .expect("new candidate missing");
    assert!(old_pos < new_pos, "oldest-first order broken: {ordered:?}");

    // Slot tie-break: two candidates on the same object, created back-to-back
    // (same ms), fall back to the slot's canonical string. `table.alias` sorts
    // before `table.description`, so the alias comes first.
    let both = object_ref(&p, "both");
    put_item(
        &store,
        &both,
        ClaimPayload::table_alias("both_alias").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    put_item(
        &store,
        &both,
        ClaimPayload::table_description("both description").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let both_rows: Vec<&str> = queued
        .iter()
        .filter(|q| q.claim.object.object() == "both")
        .map(|q| q.claim.id.as_str())
        .collect();
    let alias_id = item_id_for(&store, &both, &KnowledgeSlot::TableAlias)
        .await
        .as_str()
        .to_string();
    let desc_id = item_id_for(&store, &both, &KnowledgeSlot::TableDescription)
        .await
        .as_str()
        .to_string();
    // `table.alias` < `table.description` lexically, so the alias precedes the
    // description when their created stamps coincide.
    let alias_pos = both_rows
        .iter()
        .position(|id| *id == alias_id)
        .expect("alias in queue");
    let desc_pos = both_rows
        .iter()
        .position(|id| *id == desc_id)
        .expect("description in queue");
    assert!(
        alias_pos < desc_pos,
        "slot tie-break broken (alias should precede description): {both_rows:?}"
    );

    // The full queue order must be identical on a second run — a queue whose
    // order shifts between runs is one a user cannot work through.
    let run_a = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        200,
    )
    .await
    .unwrap();
    let run_b = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        200,
    )
    .await
    .unwrap();
    let ids_a: Vec<ClaimId> = run_a.iter().map(|q| q.claim.id.clone()).collect();
    let ids_b: Vec<ClaimId> = run_b.iter().map(|q| q.claim.id.clone()).collect();
    assert_eq!(ids_a, ids_b, "queue order was not stable across runs");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_limit_is_respected_and_clamped_at_200() {
    let root = temp_root("queue_limit");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let live = schema_tree_for(&[]);
    // Five Pending candidates on distinct objects.
    for i in 0..5 {
        put_item(
            &store,
            &object_ref(&p, &format!("t{i}")),
            ClaimPayload::table_alias(format!("t{i}")).unwrap(),
            KnowledgeState::Pending,
        )
        .await;
    }

    // A small limit is honored exactly.
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        3,
    )
    .await
    .unwrap();
    assert_eq!(queued.len(), 3, "limit not respected");

    // A limit of zero is an empty queue, not clamped up to 200 — "nothing
    // waiting" is a legitimate answer.
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live.clone()))],
        0,
    )
    .await
    .unwrap();
    assert!(queued.is_empty(), "limit 0 returned candidates");

    // An over-large limit clamps to 200 and does not error; with only five
    // candidates present, all five come back. The clamp is a ceiling the
    // operation enforces, not a promise to return more than exists.
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(queued.len(), 5, "over-large limit misbehaved");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_reports_schema_state_for_a_changed_object() {
    let root = temp_root("queue_schema_state");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A Pending candidate whose `Column { amount, Exists }` binding is dropped
    // by the live schema — the queue must flag it stale, not current.
    put_item(
        &store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let live_dropped = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live_dropped))],
        200,
    )
    .await
    .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].schema_state,
        ContractSchemaState::Stale,
        "a candidate whose referenced column is gone must read stale"
    );

    // Same candidate against the unchanged live schema reads current.
    let live_current = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live_current))],
        200,
    )
    .await
    .unwrap();
    assert_eq!(queued[0].schema_state, ContractSchemaState::Current);

    // No live schema for the profile reads live_schema_unavailable.
    let queued = review_queue(&store, std::slice::from_ref(&p), &[], 200)
        .await
        .unwrap();
    assert_eq!(
        queued[0].schema_state,
        ContractSchemaState::LiveSchemaUnavailable
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_evidence_count_is_zero_without_an_evidence_table() {
    let root = temp_root("queue_evidence_count");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(live))],
        200,
    )
    .await
    .unwrap();
    assert_eq!(queued.len(), 1);
    // `contract_evidence` is gone by design: a successful query is not evidence
    // a business definition is true. The carried count is always zero — kept
    // on the carrier only until the presentation layer drops the field.
    assert_eq!(
        queued[0].evidence_count, 0,
        "no evidence is attached to a knowledge item"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_unopenable_store_errors_unavailable() {
    let root = temp_root("queue_unavailable");
    fs::write(root.join("blocker"), b"x").unwrap();
    let bad = root.join("blocker/state.sqlite3");
    let store = SqliteStateStore::new(&bad);

    let p = profile_a();
    let err = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), avail(SchemaTree::default()))],
        200,
    )
    .await
    .unwrap_err();
    assert_eq!(err, ContractOpError::Unavailable);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Spec D / Chunk 3: `resolve_prefix` matches `ki-…` knowledge-item ids. The
// receipt/list/show stanzas print `ki-` ids, so a resolver that only matched
// the legacy `c-` ids would silently stop resolving anything a user can see —
// the defect this chunk closes.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn resolve_prefix_matches_a_unique_ki_id_prefix() {
    let root = temp_root("resolve_prefix_unique");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    // The full id resolves, as does a short unique prefix of it.
    assert_eq!(resolve_prefix(&store, &p, id.as_str()).await.unwrap(), id);
    let short: String = id.as_str().chars().take(6).collect();
    assert_eq!(
        resolve_prefix(&store, &p, &short).await.unwrap(),
        id,
        "a short unique `ki-` prefix resolves to the item"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn resolve_prefix_refuses_ambiguous_and_missing() {
    let root = temp_root("resolve_prefix_refuse");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    // Two Pending items on distinct objects; their `ki-` ids share the `ki-`
    // marker, so a bare "ki-" prefix is ambiguous.
    put_item(
        &store,
        &object_ref(&p, "orders"),
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    put_item(
        &store,
        &object_ref(&p, "returns"),
        ClaimPayload::table_alias("returns").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let err = resolve_prefix(&store, &p, "ki-").await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::Conflict,
        "a prefix matching more than one item is ambiguous, not a guess"
    );
    // A prefix that matches nothing is NotFound, not a panic.
    let err = resolve_prefix(&store, &p, "ki-deadbeef").await.unwrap_err();
    assert_eq!(err, ContractOpError::NotFound);
    // A legacy `c-` prefix matches no `ki-` id — the receipt no longer prints
    // `c-` ids, so a stale `c-` reference refuses rather than silently writing
    // the wrong item.
    let err = resolve_prefix(&store, &p, "c-").await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::NotFound,
        "a legacy `c-` prefix matches no knowledge item"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn review_wrappers_pass_through_and_map_errors() {
    let root = temp_root("review_wrappers");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // A Pending candidate the decision ops act on, seeded on `knowledge_items`
    // (the table confirm/reject/forget now read).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;

    // confirm on an unknown id maps to NotFound. Twiddle the last hex char so
    // the id parses but matches no row.
    let mut chars: Vec<char> = id.as_str().chars().collect();
    if let Some(last) = chars.last_mut() {
        *last = match *last {
            '0'..='8' => char::from_u32(*last as u32 + 1).unwrap(),
            '9' | 'a'..='e' => char::from_u32(*last as u32 + 1).unwrap(),
            'f' => '0',
            _ => '0',
        };
    }
    let fake = ClaimId::parse(&chars.into_iter().collect::<String>()).unwrap();
    let err = confirm(&store, &fake).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotFound);

    // Rejecting the candidate moves it to Dismissed (rendered Rejected), and
    // it no longer renders as a candidate.
    let rejected = reject(&store, &id).await.unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Dismissed);

    // Forgetting a second item on the same object dismisses it; show against a
    // missing schema then finds nothing recallable (every item is Dismissed).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let second = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;
    forget(&store, &second, ForgetReason::UserRequest)
        .await
        .unwrap();
    assert_eq!(item_state(&store, &second).await, KnowledgeState::Dismissed);
    let shown = show(
        &store,
        &obj,
        &SchemaAvailability::Missing,
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap();
    assert!(shown.is_none(), "a dismissed-only contract shows None");

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Phase 5b-fix: the shared retrieval policy — computed-stale and the human path.
// ---------------------------------------------------------------------------
//
// `show` and `recall` route their output through one policy (`retrieval`). A
// contract computed `Stale` is dropped for the model and counted; kept for a
// human reviewer with its state and reason. `NeedsReview` is never excluded —
// only `Stale` is, and the two states must keep meaning different things.

/// A confirmed `default_time_column` claim on `created_at`, made against a
/// two-column table whose live schema then drops `created_at`, so the contract
/// aggregates to `Stale` (computed, not persisted). The claim's status stays
/// `Confirmed`; only the schema drifted.
/// Seeds an `Active` `default_time_column` item on `created_at` for `object`
/// against a *drifted* schema that dropped `created_at`, so the item reads
/// `Invalid` (the D-4 "stale" analog: `Active` state, `Invalid` computed
/// validity). Returns the item's `ki-…` id. The schema is cached in the store
/// so `show` classifies against it.
async fn seed_active_time_item_drifted(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
) -> ClaimId {
    let id = seed_active_time_item(store, object, "created_at").await;
    let drifted = schema_tree_for(&[(object.object(), table(&[("id", "bigint", false)]))]);
    store
        .upsert_schema(object.profile().as_str(), &drifted)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn show_keeps_a_stale_contract_with_state_and_claims_for_a_human() {
    let root = temp_root("show_stale_human");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item_drifted(&store, &obj).await;

    // The human-review path: `show` keeps the stale contract, its `Stale`
    // state, and the claim itself so a reviewer can act on it. The model path
    // (`ForModel`) is what drops; `contracts show` is `ForHumanReview`.
    let shown = show(
        &store,
        &obj,
        &avail(schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("a stale contract is shown to a human, not hidden");
    assert_eq!(
        shown.schema_state,
        ContractSchemaState::Stale,
        "the state is reported as stale"
    );
    let ids: Vec<ClaimId> = shown.claims.iter().map(|c| c.id.clone()).collect();
    assert!(
        ids.contains(&id),
        "the stale item is kept for a human reviewer, got {ids:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn show_for_model_drops_a_stale_contracts_claims_but_names_the_object() {
    let root = temp_root("show_stale_model");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    seed_active_time_item_drifted(&store, &obj).await;

    // The model-facing path: `contract_read` uses `ForModel`. The object is
    // still named and reported `Stale`, but the gone-column claim is not handed
    // to the model as a current fact.
    let shown = show(
        &store,
        &obj,
        &avail(schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForModel,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("the stale object is reported, not hidden");
    assert_eq!(shown.schema_state, ContractSchemaState::Stale);
    assert!(
        shown.claims.is_empty(),
        "no claims to act on for a stale object, got {}",
        shown.claims.len()
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn recall_for_model_counts_a_stale_exclusion() {
    let root = temp_root("recall_stale_count");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A confirmed `default_time_column` on `created_at`. Its D-4 binding is
    // `Column { created_at, Time }`.
    put_item(
        &store,
        &obj,
        ClaimPayload::default_time_column("created_at").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    // The model-facing recall path drops the contract whose binding the live
    // schema no longer satisfies (the bound column `created_at` is gone) and
    // counts it in `excluded_by_schema`, so a user can see why a fact they
    // remembered stopped appearing — the exclusion is not silent.
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(drifted))],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "the stale contract is dropped for the model"
    );
    assert!(
        outcome.diagnostics.excluded_by_schema >= 1,
        "the stale exclusion is counted: {:?}",
        outcome.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// P2: bulk store reads — recall over many objects.
//
// The N+1 fix moved recall and the queue from one store round trip per object
// to a bounded number per profile. What remains here is the property the bulk
// path was named for: recall over several objects is correct under it. The
// reconcile-atomicity test that lived here was deleted in Chunk 5 with the
// reconciler itself (staleness is computed at read time, never persisted).
// ---------------------------------------------------------------------------

/// Recall over many objects of one profile returns every match — the bulk
/// `knowledge_for_profile` path groups items across objects without a query
/// per object, and selection still ranks and admits them as the per-object loop
/// did. Twenty objects is well under the object cap, so all reach selection.
#[tokio::test]
async fn recall_over_many_objects_returns_every_match() {
    let root = temp_root("recall_many_objects");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let names: Vec<String> = (0..20).map(|i| format!("orders{i}")).collect();
    for name in &names {
        let obj = object_ref(&p, name);
        put_item(
            &store,
            &obj,
            ClaimPayload::table_alias(name.as_str()).unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }

    // The live schema names all twenty so each reads `current` — staleness is
    // not what this test exercises, and a stale match would be dropped by the
    // model-facing policy before the object bound applied, contaminating the
    // count. A term that matches every object by name selects all twenty.
    let tables: Vec<(&str, Table)> = names
        .iter()
        .map(|n| (n.as_str(), table(&[("id", "bigint", false)])))
        .collect();
    let schema = schema_tree_for(&tables);
    let terms: Vec<String> = vec!["orders".to_string()];
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &terms,
            true,
            RecallBounds {
                max_objects: 20,
                max_claims_per_object: 12,
                max_bytes: 16384,
            },
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        20,
        "bulk recall must return every matching object, got {}",
        outcome.contracts.len()
    );
    let selected: Vec<String> = outcome
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    for name in &names {
        assert!(selected.contains(name), "missing {name} in {selected:?}");
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// P1 / freshness: model vs human path against a stale-by-age cached schema.
//
// A cached schema older than the model bound (24h) must not classify a claim
// `Current` on the model path — "current" must mean "matches the schema now",
// not "matches whatever we last wrote down". It reads `LiveSchemaUnavailable`,
// so the contract is *kept* and labelled (not silently dropped as `Stale`, and
// not silently read as `Current`). The human-review path (`contracts list`/
// `show`/`queue`) is unbounded: the same stale-by-age cache classifies
// normally, because a reviewer is not asked to trust a query built on their
// own contracts. These exercise the policy branch in `recall` directly; the
// pure age comparison is unit-tested in `availability`.
// ---------------------------------------------------------------------------

/// A confirmed `default_time_column` item whose binding (`Column{column, Time}`)
/// the matching `table` satisfies, so a fresh cache reads `Current`. Only the
/// cache's *age* varies between the two paths.
async fn seed_current_time_column(
    store: &SqliteStateStore,
    obj: &DatabaseObjectRef,
    _table: &Table,
    column: &str,
) {
    put_item(
        store,
        obj,
        ClaimPayload::default_time_column(column).unwrap(),
        KnowledgeState::Active,
    )
    .await;
}

#[tokio::test]
async fn model_path_treats_a_stale_by_age_cache_as_live_schema_unavailable() {
    let root = temp_root("freshness_model_stale_by_age");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let table = table(&[("id", "bigint", false), ("created_at", "timestamp", false)]);
    seed_current_time_column(&store, &obj, &table, "created_at").await;

    // The cache matches the claim, but was observed 25h ago — past the 24h
    // bound. `now` is 25h after observation (0).
    let stale_by_age = SchemaAvailability::available(schema_tree_for(&[("orders", table)]), 0);
    let now = 25 * 60 * 60 * 1000;
    let outcome = recall(
        &store,
        recall_request_with_freshness(
            std::slice::from_ref(&p),
            &[(p.clone(), stale_by_age)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
            now,
            RetrievalPolicy::ForModel,
        ),
    )
    .await;
    // The contract is NOT dropped (only computed-`Stale` is dropped): it
    // survives, classified `LiveSchemaUnavailable` so the model sees the claim
    // labelled, never silently `Current`.
    assert_eq!(
        outcome.contracts.len(),
        1,
        "a stale-by-age contract is kept for the model, labelled — not dropped"
    );
    assert_eq!(
        outcome.contracts[0].schema_state,
        ContractSchemaState::LiveSchemaUnavailable,
        "a stale-by-age cache cannot vouch for currency on the model path"
    );
    assert_eq!(
        outcome.diagnostics.excluded_by_schema, 0,
        "a stale-by-age contract is not excluded as stale: {:?}",
        outcome.diagnostics
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn human_path_uses_a_stale_by_age_cache_to_classify_current() {
    let root = temp_root("freshness_human_stale_by_age");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let table = table(&[("id", "bigint", false), ("created_at", "timestamp", false)]);
    seed_current_time_column(&store, &obj, &table, "created_at").await;

    // Same 25h-old cache, but the human-review path is unbounded: the cache
    // classifies normally. `contracts list`/`show`/`queue` rely on this — a
    // reviewer looking at their own contracts is not asked to trust a query
    // built on them.
    let stale_by_age = SchemaAvailability::available(schema_tree_for(&[("orders", table)]), 0);
    let now = 25 * 60 * 60 * 1000;
    let outcome = recall(
        &store,
        recall_request_with_freshness(
            std::slice::from_ref(&p),
            &[(p.clone(), stale_by_age)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
            now,
            RetrievalPolicy::ForHumanReview,
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        1,
        "the human path shows the contract despite the stale-by-age cache"
    );
    assert_eq!(
        outcome.contracts[0].schema_state,
        ContractSchemaState::Current,
        "the human path classifies against the stale-by-age cache as current"
    );

    let _ = fs::remove_dir_all(root);
}

/// Like [`recall_request`] but lets a test name the policy and the "now" the
/// freshness bound compares against. The default helper fixes both for the
/// common (model-path, fresh) case; the freshness tests need to vary them.
fn recall_request_with_freshness<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    allow_database_context: bool,
    bounds: RecallBounds,
    now_unix_ms: i64,
    policy: RetrievalPolicy,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context,
        schemas,
        now_unix_ms,
        bounds,
        recall_mode: RecallMode::Confirmed,
        admit_candidate: None,
        policy,
    }
}

// ---------------------------------------------------------------------------
// P1 wiring: confirming a stale claim revalidates it against the cached
// schema (the claim's own profile), so it reads Current on the next read
// instead of bouncing back to Stale. See `contract_revalidate.rs` for the
// store-layer behaviour these exercise through the `confirm` wrapper.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn confirm_revalidates_a_stale_claim_against_the_cached_schema() {
    let root = temp_root("confirm_revalidates");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // Under D-4 a "stale" fact is `Active` (state) reading `Invalid` (computed)
    // against a drifted schema. Seed an Active `default_time_column` on
    // `created_at`, then cache a schema that has it — so the binding reads
    // `Valid` and confirm has a live table to revalidate against.
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    let base = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("created_at", "timestamp", false)]),
    )]);
    store.upsert_schema(p.as_str(), &base).await.unwrap();

    // Confirming revalidates: it refreshes the binding/fingerprint version and
    // keeps the item `Active` against the live schema, so the next read is
    // `Current` (Valid). The item was already Active — the point is that
    // confirm re-checks against the cached schema rather than trusting a stored
    // verdict, and a valid binding reads Valid after.
    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    // The next read classifies it Current: the binding validates against the
    // cached schema.
    let shown = show(
        &store,
        &obj,
        &avail(base),
        RetrievalPolicy::ForHumanReview,
        FRESH_NOW,
    )
    .await
    .unwrap()
    .expect("a confirmed-against-schema contract is shown");
    assert_eq!(
        shown.schema_state,
        ContractSchemaState::Current,
        "a revalidated item reads Current, not Stale"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_refuses_a_stale_claim_with_no_cached_schema() {
    let root = temp_root("confirm_no_schema");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // An Active fact (the "stale"-analog) with no cached schema: confirm is a
    // re-verification, and there is nothing to verify against — refuse, do not
    // rubber-stamp a belief we cannot check.
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    store.invalidate_schema(p.as_str()).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::SchemaUnavailable,
        "an existing fact cannot be reconfirmed without a schema"
    );
    // The item is unchanged — still Active, nothing written.
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_refuses_a_stale_claim_whose_referenced_column_is_gone() {
    let root = temp_root("confirm_column_gone");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;

    // Cache the drifted schema — `orders` still exists, but `created_at` is
    // gone. The item's `Column { created_at, Time }` binding reads `Invalid`,
    // so confirm must refuse rather than revive a fact whose dependency died.
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &drifted).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::ColumnGone,
        "do not revive a fact whose referenced column is gone"
    );
    // The item is unchanged — still Active, nothing written.
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn confirm_a_candidate_is_status_only_and_needs_no_schema() {
    let root = temp_root("confirm_candidate");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // A `Pending` candidate with no cached schema. Confirming it is a user
    // assertion — the user is the source, so it does not need a schema, and
    // must not require one. The item will read `SchemaUnavailable` post-confirm
    // (honest), never a false `Current`.
    store.invalidate_schema(p.as_str()).await.unwrap();
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;

    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- Absence is not evidence of absence. An empty cached schema (the no-op
// --- sentinel a fresh store writes — `databases` empty) carries no real schema
// --- information, so it is "no usable schema", not "the table is gone". This
// --- is the same class as review item #29: the old `confirm` ran `find_table`
// --- against the empty tree, got `None`, and returned `ObjectGone` — reading an
// --- empty cache as proof the object no longer exists. `store_at` caches
// --- exactly this empty default, so this is the state `/confirm` lands in
// --- before any `connection schema --refresh`.
#[tokio::test]
async fn confirm_candidate_against_an_empty_cached_schema_succeeds_not_object_gone() {
    let root = temp_root("confirm_empty_schema_candidate");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // `store_at` cached `SchemaTree::default()` (empty `databases`) — leave it.
    // An empty tree is "no schema", not "every object is gone".
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("the orders table").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;

    let confirmed = confirm(&store, &id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- The same empty cache against an `Active` fact (a re-verification) refuses
// --- `SchemaUnavailable`, not `ObjectGone`: there is nothing to verify
// --- against, and an empty cache is not proof the table is gone. The asymmetry
// --- the D-3 translation preserves — a candidate needs no schema; a
// --- re-verification does — holds for an empty cache just as it does for a
// --- missing one.
#[tokio::test]
async fn confirm_active_against_an_empty_cached_schema_is_schema_unavailable_not_object_gone() {
    let root = temp_root("confirm_empty_schema_active");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    // Cache an *empty* tree (not `invalidate_schema` → `Missing`): an empty
    // `Available` tree is the case that used to read `ObjectGone`.
    store
        .upsert_schema(p.as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::SchemaUnavailable,
        "an empty cache is no schema to verify against, not proof the table is gone"
    );
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- A *populated* schema that lacks the object's table is genuine evidence
// --- the table is gone: `ObjectGone` for either state. This is the case that
// --- stays `ObjectGone` after the empty-cache fix — the cache actually says
// --- something, and what it says is "no such table".
#[tokio::test]
async fn confirm_against_a_populated_schema_lacking_the_table_is_object_gone() {
    let root = temp_root("confirm_populated_no_table");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    // A schema with a *different* table present — populated, but `orders` is
    // absent. The cache genuinely says `orders` is gone.
    let populated = schema_tree_for(&[("shipments", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &populated).await.unwrap();

    let err = confirm(&store, &id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::ObjectGone,
        "a populated schema that lacks the table is evidence the table is gone"
    );
    assert_eq!(item_state(&store, &id).await, KnowledgeState::Active);

    let _ = fs::remove_dir_all(root);
}

// --- Confirming a fact whose dependency is gone refuses with a message naming
// --- the obstacle and the repair, rather than reporting a conflict with a
// --- claim that does not exist. Found by running the binary: both refusals
// --- rendered identically under the legacy store; the D-4 refusal keeps the
// --- same message.
#[tokio::test]
async fn confirming_a_stale_claim_names_the_missing_column_and_the_repair() {
    let root = temp_root("confirm-colgone");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let id = seed_active_time_item(&store, &obj, "created_at").await;
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    store.upsert_schema(p.as_str(), &drifted).await.unwrap();

    let error = confirm(&store, &id).await.unwrap_err();
    assert_eq!(error, ContractOpError::ColumnGone);
    let rendered = error.to_string();
    assert!(
        rendered.contains("column"),
        "names the obstacle: {rendered}"
    );
    assert!(
        rendered.contains("edit") && rendered.contains("forget"),
        "names both repairs: {rendered}"
    );
    assert!(
        !rendered.contains("conflict"),
        "must not claim a conflict with another claim: {rendered}"
    );
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// P0: term matching must bridge an ordinary English plural to the singular
// object it names. A user asks "how many rentals…" for the table `rental`;
// `rentals` is longer than `rental` so it can never be a substring of the
// qualified name, and a confirmed claim about that table never reached the
// model. These exercise selection's tier-3 match through the full `recall`
// path — `best_tier` is private to `selection.rs`, so recall is the oracle.
// ---------------------------------------------------------------------------

/// Regression (spec §4.1): a plural prompt term selects the singular table it
/// names. `rentals` must match the object `rental`. Fails before the fix.
#[tokio::test]
async fn plural_prompt_term_selects_the_singular_table() {
    let root = temp_root("plural_selects_singular");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "rental");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("one row per rental").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("rental", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rentals".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        1,
        "plural term `rentals` must select the singular `rental` table, got {}",
        outcome.contracts.len()
    );
    assert_eq!(outcome.contracts[0].object, obj);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.2: the singular still selects — the fix must not regress the plain
/// case. Term `rental` selects the `rental` table.
#[tokio::test]
async fn singular_term_still_selects() {
    let root = temp_root("singular_still_selects");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let obj = object_ref(&p, "rental");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("one row per rental").unwrap(),
        KnowledgeState::Active,
    )
    .await;

    let schema = schema_tree_for(&[("rental", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rental".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, obj);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.3: a non-plural word ending in `s` is not mangled into a wrong match.
/// `status`, `address`, `staff` each name a table; none must be singularized
/// (`status` not → `statu`, `address` not → `addres`, `staff` is `s`-free) and
/// each must still match its own name, and only its own.
#[tokio::test]
async fn irregular_s_words_are_not_mangled() {
    let root = temp_root("irregular_s_words");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    // Three objects whose names end in `s` or would be mis-singularized.
    let status = object_ref(&p, "status");
    let address = object_ref(&p, "address");
    let staff = object_ref(&p, "staff");
    for obj in [&status, &address, &staff] {
        put_item(
            &store,
            obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let schema = schema_tree_for(&[
        ("status", table(&[("id", "bigint", false)])),
        ("address", table(&[("id", "bigint", false)])),
        ("staff", table(&[("id", "bigint", false)])),
    ]);

    // Each term selects exactly its own table — none is mis-singularized into
    // selecting another (e.g. `status` must not collapse to `statu` and so fail
    // to match, nor match `address`/`staff`).
    for (term, want) in [
        ("status", &status),
        ("address", &address),
        ("staff", &staff),
    ] {
        let outcome = recall(
            &store,
            recall_request(
                std::slice::from_ref(&p),
                &[(p.clone(), avail(schema.clone()))],
                &[term.to_string()],
                true,
                RecallBounds::defaults(),
            ),
        )
        .await;
        assert_eq!(
            outcome.contracts.len(),
            1,
            "term `{term}` should select exactly one object, got {}",
            outcome.contracts.len()
        );
        assert_eq!(
            outcome.contracts[0].object, *want,
            "term `{term}` selected the wrong object"
        );
    }

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.4: over-matching does not widen. A term that names no part of an
/// object must not select it. Concretely, the schema name `public` no longer
/// selects every table in the `public` schema (the old `qn.contains("public")`
/// did), and a plural term for one table does not pull in an unrelated table
/// that shares no name segment.
#[tokio::test]
async fn term_naming_no_part_of_an_object_does_not_select_it() {
    let root = temp_root("no_overmatch");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let rental = object_ref(&p, "rental");
    let customer = object_ref(&p, "customer");
    for obj in [&rental, &customer] {
        put_item(
            &store,
            obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    // Both objects live in the `public` schema (object_ref hard-codes it).
    let schema = schema_tree_for(&[
        ("rental", table(&[("id", "bigint", false)])),
        ("customer", table(&[("id", "bigint", false)])),
    ]);

    // `public` names the schema, not either object — under the old
    // `qn.contains("public")` it matched both; it must now match neither.
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema.clone()))],
            &["public".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        outcome.contracts.len(),
        0,
        "schema-name term `public` must not select objects in the public schema, got {}",
        outcome.contracts.len()
    );

    // `rentals` names `rental` only — it must not also select `customer`.
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &["rentals".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    assert_eq!(outcome.contracts[0].object, rental);

    let _ = fs::remove_dir_all(root);
}

/// Spec §4.5: determinism — the same prompt selects the same objects in the
/// same order across runs. Selection is pure over the request and the store;
/// running the same recall twice must yield identical object sequences.
#[tokio::test]
async fn same_prompt_selects_the_same_objects_in_the_same_order() {
    let root = temp_root("selection_determinism");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    // Three objects all matched by the term `orders` (each name contains it),
    // so ranking is exercised, not just admission.
    let names = ["orders_alpha", "orders_beta", "orders_gamma"];
    for name in &names {
        let obj = object_ref(&p, name);
        put_item(
            &store,
            &obj,
            ClaimPayload::table_description("a table").unwrap(),
            KnowledgeState::Active,
        )
        .await;
    }
    let tables: Vec<(&str, Table)> = names
        .iter()
        .map(|n| (*n, table(&[("id", "bigint", false)])))
        .collect();
    let schema = schema_tree_for(&tables);
    let terms: Vec<String> = vec!["orders".to_string()];
    let bounds = RecallBounds {
        max_objects: 10,
        max_claims_per_object: 12,
        max_bytes: 16384,
    };

    // Two identical recalls against the same store — selection is pure over the
    // request, so the object sequence must be identical, not merely the set.
    let a = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema.clone()))],
            &terms,
            true,
            bounds,
        ),
    )
    .await;
    let b = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(schema))],
            &terms,
            true,
            bounds,
        ),
    )
    .await;

    let names_a: Vec<String> = a
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    let names_b: Vec<String> = b
        .contracts
        .iter()
        .map(|c| c.object.object().to_string())
        .collect();
    assert_eq!(
        names_a, names_b,
        "same prompt must select the same objects in the same order"
    );
    // All three matched objects are returned (under the object cap) and ranked.
    assert_eq!(names_a.len(), 3);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Spec C — use_candidate_once: admit one candidate to recall for one turn,
// without confirming it. The admission is in-memory and request-scoped (the
// §4 decision): nothing is persisted, so a claim used once and left cannot be
// found admissible later. These tests exercise the shared operation and the
// per-claim exception in selection; no adapter calls it yet.
// ---------------------------------------------------------------------------

/// Proposes a candidate claim with `AssistantInferred` origin and no evidence,
/// returning its id. Mirrors how `contract_propose` stores a candidate: the
/// model inferred it, no human confirmed it, no observation supports it yet.
/// A recall request under `Confirmed` mode that admits one named candidate via
/// `use_candidate_once`. Everything else is the `recall_request` default: the
/// model-facing policy, fresh schemas, no explicit refs.
fn recall_admitting_one<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaAvailability)],
    terms: &'a [String],
    admit: Option<ClaimId>,
) -> RecallRequest<'a> {
    RecallRequest {
        profiles,
        explicit_refs: &[],
        terms,
        allow_database_context: true,
        schemas,
        now_unix_ms: FRESH_NOW,
        bounds: RecallBounds::defaults(),
        recall_mode: RecallMode::Confirmed,
        policy: RetrievalPolicy::ForModel,
        admit_candidate: admit,
    }
}

/// Seeds a `Pending` knowledge item (a candidate) the recall path reads, and
/// returns its store-assigned `ki-` id. Used by the recall-admission tests
/// (and by `use_candidate_once`, which now reads the same `knowledge_items`
/// table). The schema names the table and `amount` so the item's binding
/// (`Column { amount, Exists }`) classifies `current`.
async fn seed_one_pending_item(
    store: &SqliteStateStore,
) -> (ProfileIdentity, DatabaseObjectRef, ClaimId) {
    use saya_store::KnowledgeItemStore;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    store.upsert_schema(p.as_str(), &tree).await.unwrap();
    put_item(
        store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| {
                i.slot
                    == KnowledgeSlot::ColumnDescription {
                        column: "amount".into(),
                    }
            })
            .expect("pending item stored")
            .id,
    )
    .expect("ki id");
    (p, obj, id)
}

/// 1. Using a candidate once leaves its status `Candidate` and its origin
/// `AssistantInferred`. The operation writes nothing to the store — it cannot
/// promote, and a user who uses one and walks away must find it unchanged.
#[tokio::test]
async fn use_candidate_once_leaves_status_and_origin_unchanged() {
    let root = temp_root("use_once_unchanged");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    // A live `Pending` candidate the operation admits, seeded on the
    // `knowledge_items` table `use_candidate_once` now reads.
    put_item(
        &store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(
        &store,
        &obj,
        &KnowledgeSlot::ColumnDescription {
            column: "amount".into(),
        },
    )
    .await;

    let before = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item present");
    assert_eq!(before.state, KnowledgeState::Pending);
    assert_eq!(before.source, ClaimOrigin::AssistantInferred);

    use_candidate_once(&store, &id).await.unwrap();

    let after = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item still present");
    assert_eq!(
        after.state,
        KnowledgeState::Pending,
        "using a candidate must not confirm it"
    );
    assert_eq!(
        after.source,
        ClaimOrigin::AssistantInferred,
        "using a candidate must not change its origin"
    );
    // Nothing durable moved: the operation touched no row, so the timestamp
    // the store updates on every write is the item's own.
    assert_eq!(after.updated_unix_ms, before.updated_unix_ms);

    let _ = fs::remove_dir_all(root);
}

/// 2. The admitted candidate becomes recallable within the scope, under
/// `recall = "confirmed"`, where it would otherwise be excluded.
///
/// This exercises the recall-admission behaviour: it seeds a `Pending` item and
/// admits that item's id directly via the request's `admit_candidate`. The
/// `use_candidate_once` op's own refuse-non-candidate behaviour is covered by
/// the review-op tests below; both now read `knowledge_items`.
#[tokio::test]
async fn use_candidate_once_admits_it_within_the_scope() {
    let root = temp_root("use_once_admits");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, obj, id) = seed_one_pending_item(&store).await;

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    // Without the admission, the candidate is excluded under Confirmed.
    let without = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree.clone()))],
            &terms,
            None,
        ),
    )
    .await;
    assert!(
        without.contracts.is_empty(),
        "a candidate is not recallable under confirmed without admission"
    );

    // With the admission, the candidate's claim reaches the contract.
    let with = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree))],
            &terms,
            Some(id.clone()),
        ),
    )
    .await;
    assert_eq!(
        with.contracts.len(),
        1,
        "the admitted candidate is selected"
    );
    assert_eq!(with.contracts[0].object, obj);
    let admitted: Vec<ClaimId> = with.contracts[0]
        .claims
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert!(
        admitted.contains(&id),
        "the named candidate is in the contract"
    );
    // The admitted claim is still `Candidate` in the contract recall returns.
    // The render layer marks a claim `[candidate — unconfirmed]` solely from
    // `status == Candidate`, so this is the property that keeps an admitted
    // candidate indistinguishable-from-nothing-special in the prompt: being
    // chosen for one turn confers no authority (spec C §3). If this read
    // `Confirmed`, the admission would have silently promoted it.
    let stored_admitted = with.contracts[0]
        .claims
        .iter()
        .find(|c| c.id == id)
        .expect("the admitted claim is in the contract");
    assert_eq!(
        stored_admitted.status,
        ClaimStatus::Candidate,
        "the admitted claim stays Candidate — it still renders unconfirmed"
    );

    let _ = fs::remove_dir_all(root);
}

/// 3. Outside the scope it is not recallable again. The admission is
/// request-scoped: a second recall, without the admission, excludes it — even
/// on the very next call against the same store.
#[tokio::test]
async fn use_candidate_once_does_not_persist_across_recalls() {
    let root = temp_root("use_once_not_persisted");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, _obj, id) = seed_one_pending_item(&store).await;

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    let schemas = &[(p.clone(), avail(tree))][..];
    let profiles = std::slice::from_ref(&p);

    // First recall admits it.
    let first = recall(
        &store,
        recall_admitting_one(profiles, schemas, &terms, Some(id)),
    )
    .await;
    assert_eq!(first.contracts.len(), 1);

    // A second recall, with no admission, excludes it again. The admission
    // died with the first request.
    let second = recall(
        &store,
        recall_admitting_one(profiles, schemas, &terms, None),
    )
    .await;
    assert!(
        second.contracts.is_empty(),
        "the admission is request-scoped: a later recall excludes the candidate again"
    );

    let _ = fs::remove_dir_all(root);
}

/// 4. A rejected / forgotten / confirmed claim is refused with a typed error.
/// The operation admits a live `Pending` candidate only; the rest are not
/// silently no-op'd. Under D-4 a "stale" fact is `Active` (state) reading
/// `Invalid` (computed), so it is refused as a non-candidate alongside an
/// explicit `Active` (confirmed) and a `Dismissed` (rejected/forgotten).
#[tokio::test]
async fn use_candidate_once_refuses_non_candidate_claims() {
    let root = temp_root("use_once_refuses");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");

    // Dismissed (rejected/forgotten): refused, unchanged.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("rejected").unwrap(),
        KnowledgeState::Dismissed,
    )
    .await;
    let rejected_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;
    let err = use_candidate_once(&store, &rejected_id).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotACandidate, "dismissed is refused");
    assert_eq!(
        item_state(&store, &rejected_id).await,
        KnowledgeState::Dismissed,
        "the refusal changed nothing"
    );

    // Active (confirmed / the D-4 "stale" analog): refused, unchanged. An
    // Active item is already admissible by the mode, so use-once is a category
    // error, not a silent success.
    put_item(
        &store,
        &obj,
        ClaimPayload::table_description("active fact").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let active_id = item_id_for(&store, &obj, &KnowledgeSlot::TableDescription).await;
    let err = use_candidate_once(&store, &active_id).await.unwrap_err();
    assert_eq!(err, ContractOpError::NotACandidate, "active is refused");

    let _ = fs::remove_dir_all(root);
}

/// 5. An Active (confirmed) item is refused, not a no-op. The operation's
/// contract is "admit an unconfirmed candidate"; a confirmed item is already
/// admissible by the mode, so using it once is a category error. Refusing
/// (rather than silently succeeding) keeps the caller from believing it did
/// something it did not — the same fail-closed posture as the other
/// non-candidate refusals.
#[tokio::test]
async fn use_candidate_once_refuses_a_confirmed_claim() {
    let root = temp_root("use_once_confirmed");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("orders").unwrap(),
        KnowledgeState::Active,
    )
    .await;
    let confirmed_id = item_id_for(&store, &obj, &KnowledgeSlot::TableAlias).await;

    let err = use_candidate_once(&store, &confirmed_id).await.unwrap_err();
    assert_eq!(
        err,
        ContractOpError::NotACandidate,
        "a confirmed item is refused, not a silent no-op"
    );
    // And the item is untouched.
    assert_eq!(
        item_state(&store, &confirmed_id).await,
        KnowledgeState::Active
    );

    let _ = fs::remove_dir_all(root);
}

/// 6. Only the named claim is admitted; a sibling candidate on the same object
/// is not. The admission is per-claim, not per-object — the whole point of the
/// feature is to avoid the `include-candidates`-for-everything shape.
///
/// Seeds two `Pending` D-3 items on one object with distinct, valid bindings
/// (a `column_description` on `amount` and a `table_alias`), so validity does
/// not drop either — admission alone decides. See the note on
/// `use_candidate_once_admits_it_within_the_scope` for why `use_candidate_once`
/// itself is not called here.
#[tokio::test]
async fn use_candidate_once_admits_only_the_named_claim() {
    use saya_store::KnowledgeItemStore;
    let root = temp_root("use_once_only_named");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let (p, obj, admitted_id) = seed_one_pending_item(&store).await;

    // A sibling Pending item on the same object: a table alias (a `Table`
    // binding, valid against the orders table).
    put_item(
        &store,
        &obj,
        ClaimPayload::table_alias("sibling").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let sibling_id = ClaimId::parse(
        &store
            .knowledge_for_object(&obj)
            .await
            .expect("knowledge items listed")
            .into_iter()
            .find(|i| i.slot == KnowledgeSlot::TableAlias)
            .expect("sibling item stored")
            .id,
    )
    .expect("ki id");
    assert_ne!(sibling_id, admitted_id);

    let tree = schema_tree_for(&[(
        "orders",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    let terms: Vec<String> = vec!["orders".to_string()];
    let outcome = recall(
        &store,
        recall_admitting_one(
            std::slice::from_ref(&p),
            &[(p.clone(), avail(tree))],
            &terms,
            Some(admitted_id.clone()),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1, "the object is selected");
    let ids: Vec<ClaimId> = outcome.contracts[0]
        .claims
        .iter()
        .map(|c| c.id.clone())
        .collect();
    assert!(
        ids.contains(&admitted_id),
        "the named candidate is admitted"
    );
    assert!(
        !ids.contains(&sibling_id),
        "the sibling candidate is not admitted — admission is per-claim"
    );

    let _ = fs::remove_dir_all(root);
}

/// 7. Using a candidate writes nothing to the store. Using is not an
/// observation about the database; the operation touches no row, so the item's
/// state and write timestamp are its own afterwards. (`contract_evidence` is
/// gone, so "no evidence" is now structural — there is no table to write.)
#[tokio::test]
async fn use_candidate_once_creates_no_evidence() {
    let root = temp_root("use_once_no_evidence");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    put_item(
        &store,
        &obj,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        KnowledgeState::Pending,
    )
    .await;
    let id = item_id_for(
        &store,
        &obj,
        &KnowledgeSlot::ColumnDescription {
            column: "amount".into(),
        },
    )
    .await;

    let before = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item present");

    use_candidate_once(&store, &id).await.unwrap();

    let after = store
        .get_knowledge_item(id.as_str())
        .await
        .unwrap()
        .expect("item still present");
    assert_eq!(after.state, KnowledgeState::Pending);
    assert_eq!(
        after.updated_unix_ms, before.updated_unix_ms,
        "using a candidate writes nothing; the timestamp is unchanged"
    );

    let _ = fs::remove_dir_all(root);
}
