//! Integration tests for the contract-application operations layer.
//!
//! These exercise the `pub(crate)` surface that every adapter will render:
//! recall, validity, conflict detection, and the review wrappers. They are
//! in-crate (not under `tests/`) because the surface is `pub(crate)` — the
//! operations layer returns typed data for saya-cli's own adapters, not for
//! external consumers. See the spec at .claude/specs/spec-2b1-contract-operations.md.

use super::{
    ContractOpError, ContractSchemaState, RecallBounds, RecallDiagnostics, RecallMode,
    RecallRequest, confirm, edit, forget, propose, recall, reject, schema_state_for, show,
};
use saya_store::{
    ContractStore, ForgetReason, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore,
    StoredClaim,
};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, Database, DatabaseObjectKind,
    DatabaseObjectRef, ProfileIdentity, Schema, SchemaFingerprint, SchemaTree, Table,
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

fn fingerprint_for(table: &Table) -> SchemaFingerprint {
    SchemaFingerprint::of_table(DatabaseObjectKind::Table, table)
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

async fn store_at(db: &Path) -> SqliteStateStore {
    let store = SqliteStateStore::new(db);
    // Touch the pool so migrations run and the schema exists.
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();
    store
}

async fn propose_confirmed(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    fingerprint: &SchemaFingerprint,
    payload: ClaimPayload,
) -> ClaimId {
    let referenced_columns = payload.referenced_column_name_snapshots();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint.clone(),
        referenced_columns,
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

async fn confirm_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    fingerprint: &SchemaFingerprint,
    payload: ClaimPayload,
) -> ClaimId {
    let referenced_columns = payload.referenced_column_name_snapshots();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint.clone(),
        referenced_columns,
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    store.confirm_claim(&id).await.unwrap();
    id
}

/// Proposes a candidate claim and returns its id. `evidence_turns` repeats the
/// proposal once per turn ordinal, each attaching one `RepeatedObservation`
/// evidence row so the claim ends with `evidence_turns.len()` support. The
/// queue orders by that count, so this is the lever the ordering test pulls.
async fn propose_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    fingerprint: &SchemaFingerprint,
    payload: ClaimPayload,
    evidence_turns: &[u32],
) -> ClaimId {
    let turns = evidence_turns.to_vec();
    let referenced_columns = payload.referenced_column_name_snapshots();
    let request = |turn: Option<u32>| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint.clone(),
        referenced_columns: referenced_columns.clone(),
        payload: payload.clone(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: turn.map(|t| saya_store::ClaimEvidence {
            kind: saya_store::EvidenceKind::RepeatedObservation,
            session_id: Some("s1".into()),
            turn_ordinal: Some(t),
            observed_unix_ms: 10_000 + t as i64,
        }),
    };
    let first = match store
        .propose_claim(request(turns.first().copied()))
        .await
        .unwrap()
    {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    for turn in turns.into_iter().skip(1) {
        store.propose_claim(request(Some(turn))).await.unwrap();
    }
    first
}

fn recall_request<'a>(
    profiles: &'a [ProfileIdentity],
    schemas: &'a [(ProfileIdentity, SchemaTree)],
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
        bounds,
        // The existing recall tests model today's behaviour: confirmed only.
        // A test that needs `IncludeCandidates` builds its own request.
        recall_mode: RecallMode::Confirmed,
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
    let fp_a = fingerprint_for(&table(&[("id", "bigint", false)]));
    let fp_b = fingerprint_for(&table(&[("id", "int", false)]));

    let _id_a = propose_confirmed(
        &store,
        &obj_a,
        &fp_a,
        ClaimPayload::table_alias("orders").unwrap(),
    )
    .await;
    let _id_b = propose_confirmed(
        &store,
        &obj_b,
        &fp_b,
        ClaimPayload::table_alias("orders").unwrap(),
    )
    .await;

    let schema_a = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&a),
            &[(a.clone(), schema_a)],
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
            &[(b.clone(), schema_b)],
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
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));

    let _confirmed = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("orders").unwrap(),
    )
    .await;
    let req = ProposeClaim {
        object: obj.clone(),
        fingerprint: fp.clone(),
        payload: ClaimPayload::table_alias("secret_alias").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let _ = store.propose_claim(req).await.unwrap();

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
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
                c.payload.as_ref(),
                Some(ClaimPayload::TableAlias { alias, .. }) if alias == "secret_alias"
            )
        }),
        "candidate alias leaked into recall"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: bounds hold on objects, claims, and bytes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn bounds_hold_on_objects_claims_and_bytes() {
    let root = temp_root("bounds");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let objs: Vec<DatabaseObjectRef> = (0..3)
        .map(|i| object_ref(&p, &format!("orders{i}")))
        .collect();
    for obj in &objs {
        let _ = propose_confirmed(
            &store,
            obj,
            &fp,
            ClaimPayload::table_alias(obj.object()).unwrap(),
        )
        .await;
    }
    let heavy = object_ref(&p, "heavy");
    for i in 0..20 {
        let _ = propose_confirmed(
            &store,
            &heavy,
            &fp,
            ClaimPayload::table_alias(format!("a{i}")).unwrap(),
        )
        .await;
    }

    // max_objects = 2 over 3 matches -> exactly 2, truncated.
    let terms: Vec<String> = objs.iter().map(|o| o.object().to_string()).collect();
    let bounds = RecallBounds {
        max_objects: 2,
        max_claims_per_object: 12,
        max_bytes: 16384,
    };
    let schema = schema_tree_for(&[("orders0", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
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
            &[(p.clone(), schema_h.clone())],
            &["heavy".to_string()],
            true,
            bounds2,
        ),
    )
    .await;
    assert_eq!(outcome2.contracts.len(), 1);
    assert_eq!(outcome2.contracts[0].claims.len(), 3);
    assert!(outcome2.contracts[0].truncated);

    // max_bytes = 80: serialized payloads never exceed the budget, and it truncates.
    let bounds3 = RecallBounds {
        max_objects: 5,
        max_claims_per_object: 12,
        max_bytes: 80,
    };
    let outcome3 = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema_h)],
            &["heavy".to_string()],
            true,
            bounds3,
        ),
    )
    .await;
    let serialized: usize = outcome3
        .contracts
        .iter()
        .flat_map(|c| c.claims.iter())
        .map(|c| {
            serde_json::to_string(&c.payload)
                .map(|s| s.len())
                .unwrap_or(0)
        })
        .sum();
    assert!(serialized <= 80, "max_bytes exceeded: {serialized}");
    assert!(!outcome3.contracts.is_empty());
    assert!(outcome3.contracts[0].truncated);

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
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj1 = object_ref(&p, "table_one");
    let obj2 = object_ref(&p, "table_two");
    let _ = propose_confirmed(
        &store,
        &obj1,
        &fp,
        ClaimPayload::table_alias("shared").unwrap(),
    )
    .await;
    let _ = propose_confirmed(
        &store,
        &obj2,
        &fp,
        ClaimPayload::table_alias("shared").unwrap(),
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
            &[(p.clone(), schema)],
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
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let _ = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("orders").unwrap(),
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
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
            &[(p.clone(), schema)],
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
// Test 7: validity matrix (see SPEC REVIEW for the retyped/nullability deviation)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn validity_matrix() {
    let root = temp_root("validity");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);

    // Current
    let fp = fingerprint_for(&base);
    let obj_cur = object_ref(&p, "cur");
    let id_cur = propose_confirmed(
        &store,
        &obj_cur,
        &fp,
        ClaimPayload::table_alias("cur").unwrap(),
    )
    .await;
    let claim_cur = store.get_claim(&id_cur).await.unwrap().unwrap();
    let live_cur = schema_tree_for(&[("cur", base.clone())]);
    assert_eq!(
        schema_state_for(&claim_cur, Some(&live_cur)),
        ContractSchemaState::Current
    );

    // Unrelated column added -> NeedsReview
    let obj_add = object_ref(&p, "added");
    let id_add = confirm_candidate(
        &store,
        &obj_add,
        &fp,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let claim_add = store.get_claim(&id_add).await.unwrap().unwrap();
    let live_added = schema_tree_for(&[(
        "added",
        table(&[
            ("id", "bigint", false),
            ("amount", "numeric", false),
            ("note", "text", true),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim_add, Some(&live_added)),
        ContractSchemaState::NeedsReview
    );

    // Referenced column removed -> Stale
    let obj_rm = object_ref(&p, "removed");
    let id_rm = confirm_candidate(
        &store,
        &obj_rm,
        &fingerprint_for(&base),
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let claim_rm = store.get_claim(&id_rm).await.unwrap().unwrap();
    let live_rm = schema_tree_for(&[("removed", table(&[("id", "bigint", false)]))]);
    assert_eq!(
        schema_state_for(&claim_rm, Some(&live_rm)),
        ContractSchemaState::Stale
    );

    // Referenced column retyped (name kept) -> NeedsReview (SPEC REVIEW deviation)
    let obj_rt = object_ref(&p, "retyped");
    let id_rt = confirm_candidate(
        &store,
        &obj_rt,
        &fingerprint_for(&base),
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let claim_rt = store.get_claim(&id_rt).await.unwrap().unwrap();
    let live_rt = schema_tree_for(&[(
        "retyped",
        table(&[
            ("id", "bigint", false),
            ("amount", "double precision", false),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim_rt, Some(&live_rt)),
        ContractSchemaState::NeedsReview,
        "retyped referenced column should be NeedsReview (SPEC REVIEW deviation)"
    );

    // Table absent -> Stale
    let obj_absent = object_ref(&p, "absent");
    let id_absent = propose_confirmed(
        &store,
        &obj_absent,
        &fp,
        ClaimPayload::table_alias("absent").unwrap(),
    )
    .await;
    let claim_absent = store.get_claim(&id_absent).await.unwrap().unwrap();
    let live_absent = schema_tree_for(&[]);
    assert_eq!(
        schema_state_for(&claim_absent, Some(&live_absent)),
        ContractSchemaState::Stale
    );

    // No live schema -> LiveSchemaUnavailable
    assert_eq!(
        schema_state_for(&claim_cur, None),
        ContractSchemaState::LiveSchemaUnavailable
    );

    // Older fingerprint version -> NeedsReview, never Current
    let old = SchemaFingerprint::from_parts(2, claim_cur.schema_fingerprint.as_str()).unwrap();
    let claim_old = StoredClaim {
        schema_fingerprint: old,
        ..claim_cur.clone()
    };
    assert_eq!(
        schema_state_for(&claim_old, Some(&live_cur)),
        ContractSchemaState::NeedsReview,
        "older fingerprint version must be NeedsReview, never Current"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 8: two TableGrain claims conflict but both are returned
// ---------------------------------------------------------------------------
#[tokio::test]
async fn two_table_grain_claims_conflict_but_both_returned() {
    let root = temp_root("grain_conflict");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let id1 = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_grain("one row per order").unwrap(),
    )
    .await;
    let id2 = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_grain("one row per order line").unwrap(),
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(outcome.contracts.len(), 1);
    let contract = &outcome.contracts[0];
    let grain_conflicts: Vec<&ContractConflict> = contract
        .conflicts
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
    assert_eq!(contract.claims.len(), 2);

    let _ = fs::remove_dir_all(root);
}

/// SPEC REVIEW companion: two TableDescription claims do NOT conflict.
#[tokio::test]
async fn two_table_description_claims_do_not_conflict() {
    let root = temp_root("desc_no_conflict");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let _id1 = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_description("sales fact table").unwrap(),
    )
    .await;
    let _id2 = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_description("updated nightly").unwrap(),
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
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
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let _ = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_description(format!("orders {SENTINEL} facts")).unwrap(),
    )
    .await;

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome: RecallOutcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
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
#[tokio::test]
async fn forgotten_claim_disappears_from_recall() {
    let root = temp_root("forget_disappears");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let keep = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("keep").unwrap(),
    )
    .await;
    let gone = propose_confirmed(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("gone").unwrap(),
    )
    .await;
    store
        .forget_claim(&gone, ForgetReason::Obsolete)
        .await
        .unwrap();

    let schema = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), schema)],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    let contract = &outcome.contracts[0];
    let ids: Vec<ClaimId> = contract.claims.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&keep), "kept claim should remain");
    assert!(!ids.contains(&gone), "forgotten claim should not appear");

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
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    // A confirmed claim and a forgotten one must not appear; a candidate must.
    let _confirmed = confirm_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("confirmed_alias").unwrap(),
    )
    .await;
    let cand_id = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("cand_alias").unwrap(),
        &[1],
    )
    .await;
    let forgotten = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("forgotten_alias").unwrap(),
        &[1],
    )
    .await;
    store
        .forget_claim(&forgotten, ForgetReason::Obsolete)
        .await
        .unwrap();

    let queued = review_queue(&store, std::slice::from_ref(&p), &[(p.clone(), live)], 200)
        .await
        .unwrap();

    let ids: Vec<ClaimId> = queued.iter().map(|q| q.claim.id.clone()).collect();
    assert!(ids.contains(&cand_id), "candidate missing from queue");
    // Every queued claim is a candidate — neither confirmed nor forgotten
    // leaks in. Reading each back lets the assertion stay sync inside `any`.
    for id in &ids {
        let claim = store.get_claim(id).await.unwrap().unwrap();
        assert_eq!(
            claim.status,
            ClaimStatus::Candidate,
            "non-candidate claim appeared in the queue"
        );
    }
    assert!(
        !ids.contains(&forgotten),
        "a forgotten claim appeared in the queue"
    );
    // The queue carries one entry per candidate, not per claim-status.
    assert_eq!(queued.len(), 1);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_orders_most_evidence_then_oldest_then_id() {
    let root = temp_root("queue_order");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    // Three distinct objects so three candidates with distinct claim ids.
    let live = schema_tree_for(&[
        ("a", table(&[("id", "bigint", false)])),
        ("b", table(&[("id", "bigint", false)])),
        ("c", table(&[("id", "bigint", false)])),
    ]);

    // most: 3 evidence, middle: 2, least: 1.
    let _most = propose_candidate(
        &store,
        &object_ref(&p, "a"),
        &fp,
        ClaimPayload::table_alias("a").unwrap(),
        &[1, 2, 3],
    )
    .await;
    let _middle = propose_candidate(
        &store,
        &object_ref(&p, "b"),
        &fp,
        ClaimPayload::table_alias("b").unwrap(),
        &[1, 2],
    )
    .await;
    let _least = propose_candidate(
        &store,
        &object_ref(&p, "c"),
        &fp,
        ClaimPayload::table_alias("c").unwrap(),
        &[1],
    )
    .await;

    let first = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live.clone())],
        200,
    )
    .await
    .unwrap();
    assert_eq!(first.len(), 3);
    // Most evidence first: a (3) before b (2) before c (1).
    let by_object: Vec<&str> = first.iter().map(|q| q.claim.object.object()).collect();
    assert_eq!(
        by_object,
        vec!["a", "b", "c"],
        "evidence order broken: {by_object:?}"
    );

    // Tie-break by oldest: two claims with equal evidence (1) on different
    // objects, created in sequence, must come back oldest-first. A short sleep
    // makes the millisecond timestamps differ so the secondary key is what
    // decides — without it both land in the same ms and the test would only
    // exercise the tertiary (id) key.
    let live_tb = schema_tree_for(&[
        ("old", table(&[("id", "bigint", false)])),
        ("new", table(&[("id", "bigint", false)])),
    ]);
    let _old = propose_candidate(
        &store,
        &object_ref(&p, "old"),
        &fp,
        ClaimPayload::table_alias("old").unwrap(),
        &[1],
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let _new = propose_candidate(
        &store,
        &object_ref(&p, "new"),
        &fp,
        ClaimPayload::table_alias("new").unwrap(),
        &[1],
    )
    .await;

    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_tb)],
        200,
    )
    .await
    .unwrap();
    // The store now holds a/b/c (evidence 3/2/1) plus old/new (evidence 1 each).
    // The equal-evidence pair sorts oldest-first; assert the relative order of
    // the two rather than the whole list, since a/b/c interleave by evidence.
    let ordered: Vec<&str> = queued.iter().map(|q| q.claim.object.object()).collect();
    let old_pos = ordered
        .iter()
        .position(|o| *o == "old")
        .expect("old candidate missing");
    let new_pos = ordered
        .iter()
        .position(|o| *o == "new")
        .expect("new candidate missing");
    assert!(
        old_pos < new_pos,
        "oldest-first tie-break broken: {ordered:?}"
    );

    // The full queue order must be identical on a second run — a queue whose
    // order shifts between runs is one a user cannot work through. Both runs
    // see the same five candidates now that the store holds a/b/c and old/new.
    let live_all = schema_tree_for(&[
        ("a", table(&[("id", "bigint", false)])),
        ("b", table(&[("id", "bigint", false)])),
        ("c", table(&[("id", "bigint", false)])),
        ("old", table(&[("id", "bigint", false)])),
        ("new", table(&[("id", "bigint", false)])),
    ]);
    let run_a = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_all.clone())],
        200,
    )
    .await
    .unwrap();
    let run_b = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_all)],
        200,
    )
    .await
    .unwrap();
    let ids_a: Vec<ClaimId> = run_a.iter().map(|q| q.claim.id.clone()).collect();
    let ids_b: Vec<ClaimId> = run_b.iter().map(|q| q.claim.id.clone()).collect();
    assert_eq!(ids_a, ids_b, "queue order was not stable across runs");
    // And it still begins with the most-evidence candidate.
    assert_eq!(run_a[0].claim.object.object(), "a");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn queue_limit_is_respected_and_clamped_at_200() {
    let root = temp_root("queue_limit");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let live = schema_tree_for(&[]);
    // Five candidates, equal evidence so the id tie-break orders them.
    for i in 0..5 {
        let obj = object_ref(&p, &format!("t{i}"));
        let _ = propose_candidate(
            &store,
            &obj,
            &fp,
            ClaimPayload::table_alias(format!("t{i}")).unwrap(),
            &[1],
        )
        .await;
    }

    // A small limit is honored exactly.
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live.clone())],
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
        &[(p.clone(), live.clone())],
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
        &[(p.clone(), live)],
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
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let fp = fingerprint_for(&base);
    let obj = object_ref(&p, "orders");
    // Candidate proposed against the base schema, then the live schema drops a
    // referenced column — the queue must flag it stale, not current.
    let _id = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::column_description("amount", "how much").unwrap(),
        &[1],
    )
    .await;
    let live_dropped = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_dropped)],
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
    let live_current = schema_tree_for(&[("orders", base.clone())]);
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_current)],
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
async fn queue_evidence_count_reflects_attached_evidence() {
    let root = temp_root("queue_evidence_count");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    // Three distinct evidence rows (distinct turn ordinals) -> count 3.
    let _id = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("orders").unwrap(),
        &[1, 2, 3],
    )
    .await;
    let queued = review_queue(&store, std::slice::from_ref(&p), &[(p.clone(), live)], 200)
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].evidence_count, 3);

    // The count the queue carries must match the store's own count read.
    assert_eq!(
        queued[0].evidence_count,
        store.evidence_count(&queued[0].claim.id).await.unwrap()
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
        &[(p.clone(), SchemaTree::default())],
        200,
    )
    .await
    .unwrap_err();
    assert_eq!(err, ContractOpError::Unavailable);

    let _ = fs::remove_dir_all(root);
}
#[tokio::test]
async fn review_wrappers_pass_through_and_map_errors() {
    let root = temp_root("review_wrappers");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[("id", "bigint", false)]));
    let obj = object_ref(&p, "orders");

    let req = ProposeClaim {
        object: obj.clone(),
        fingerprint: fp.clone(),
        payload: ClaimPayload::table_alias("orders").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match propose(&store, req).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };

    // confirm on an unknown id maps to NotFound
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

    // edit on the real claim succeeds
    let edited = edit(&store, &id, ClaimPayload::table_alias("orders2").unwrap())
        .await
        .unwrap();
    assert_eq!(edited.status, ClaimStatus::Confirmed);

    // propose a candidate, then reject it -> Rejected, and no longer recallable.
    let cand = ProposeClaim {
        object: obj.clone(),
        fingerprint: fp.clone(),
        payload: ClaimPayload::table_alias("cand").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let cand_id = match propose(&store, cand).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let rejected = reject(&store, &cand_id).await.unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);

    // forget then show: nothing recallable remains
    forget(&store, &id, ForgetReason::UserRequest)
        .await
        .unwrap();
    let shown = show(&store, &obj, &fp).await.unwrap();
    assert!(shown.is_none(), "forgotten-only contract should show None");

    let _ = fs::remove_dir_all(root);
}
