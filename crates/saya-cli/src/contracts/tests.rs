//! Integration tests for the contract-application operations layer.
//!
//! These exercise the `pub(crate)` surface that every adapter will render:
//! recall, validity, conflict detection, and the review wrappers. They are
//! in-crate (not under `tests/`) because the surface is `pub(crate)` — the
//! operations layer returns typed data for saya-cli's own adapters, not for
//! external consumers. See the spec at .claude/specs/spec-2b1-contract-operations.md.

use super::{
    ContractOpError, ContractSchemaState, RecallBounds, RecallDiagnostics, RecallMode,
    RecallRequest, RetrievalPolicy, confirm, edit, forget, propose, recall, reject,
    schema_state_for, show,
};
use saya_store::{
    ContractEventKind, ContractStore, ForgetReason, ProposeClaim, ProposeOutcome, SchemaStore,
    SqliteStateStore, StoredClaim,
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
// Test 7: the 5b typed schema-drift matrix
// ---------------------------------------------------------------------------
//
// One assertion per row of the §1 rule table in spec-5b-drift-rule.md. The rows
// that exercise *type* and *nullability* drift need a **known** column
// snapshot — a snapshot whose `data_type` and `nullable` were actually
// observed at claim time — so they are proposed against the live table they
// describe via `referenced_column_snapshots`. The pre-5a / unknown-snapshot
// row uses the name-only path (`referenced_column_name_snapshots`), whose
// empty `data_type` the reconciler treats as unknown rather than matched.
#[tokio::test]
async fn validity_matrix() {
    let root = temp_root("validity");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);

    // A claim proposed against the live `base` table carries a *known*
    // snapshot per referenced column: `amount` -> `numeric`, not-null.
    async fn known_claim(
        store: &SqliteStateStore,
        object: &DatabaseObjectRef,
        live: &Table,
        payload: ClaimPayload,
    ) -> StoredClaim {
        let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, live);
        let referenced = payload.referenced_column_snapshots(live);
        let req = ProposeClaim {
            object: object.clone(),
            fingerprint: fp.clone(),
            payload,
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Confirmed,
            evidence: None,
            referenced_columns: referenced,
        };
        let id = match store.propose_claim(req).await.unwrap() {
            ProposeOutcome::Stored(id) => id,
            other => panic!("expected Stored, got {other:?}"),
        };
        store.get_claim(&id).await.unwrap().unwrap()
    }

    // Row 1: fingerprint matches -> Current.
    let obj_cur = object_ref(&p, "cur");
    let claim_cur = known_claim(
        &store,
        &obj_cur,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_cur = schema_tree_for(&[("cur", base.clone())]);
    assert_eq!(
        schema_state_for(&claim_cur, Some(&live_cur)),
        ContractSchemaState::Current
    );

    // Row 2: an unrelated column added -> NeedsReview, not Stale. The claim
    // depends on `amount`, which is unchanged; `note` is none of its business.
    let obj_add = object_ref(&p, "added");
    let claim_add = known_claim(
        &store,
        &obj_add,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
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
        ContractSchemaState::NeedsReview,
        "an unrelated addition must not invalidate the claim"
    );

    // Row 3: a referenced column removed -> Stale.
    let obj_rm = object_ref(&p, "removed");
    let claim_rm = known_claim(
        &store,
        &obj_rm,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_rm = schema_tree_for(&[("removed", table(&[("id", "bigint", false)]))]);
    assert_eq!(
        schema_state_for(&claim_rm, Some(&live_rm)),
        ContractSchemaState::Stale
    );

    // Row 4: a referenced column renamed -> Stale (absent by name).
    let obj_rn = object_ref(&p, "renamed");
    let claim_rn = known_claim(
        &store,
        &obj_rn,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_rn = schema_tree_for(&[(
        "renamed",
        table(&[("id", "bigint", false), ("amt", "numeric", false)]),
    )]);
    assert_eq!(
        schema_state_for(&claim_rn, Some(&live_rn)),
        ContractSchemaState::Stale,
        "a renamed referenced column is absent by name -> Stale"
    );

    // Row 5: a referenced column retyped (name kept) -> Stale. This is the
    // case that was impossible before 5a — name it so a regression is obvious.
    let obj_rt = object_ref(&p, "retyped");
    let claim_rt = known_claim(
        &store,
        &obj_rt,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_rt = schema_tree_for(&[(
        "retyped",
        table(&[
            ("id", "bigint", false),
            ("amount", "double precision", false),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim_rt, Some(&live_rt)),
        ContractSchemaState::Stale,
        "a retyped referenced column is Stale (this case was impossible before 5a)"
    );

    // Row 6: a referenced column became nullable -> Stale. Gaining NULLs can
    // silently change what a claim about it means.
    let obj_null = object_ref(&p, "nullable");
    let claim_null = known_claim(
        &store,
        &obj_null,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_null = schema_tree_for(&[(
        "nullable",
        table(&[("id", "bigint", false), ("amount", "numeric", true)]),
    )]);
    assert_eq!(
        schema_state_for(&claim_null, Some(&live_null)),
        ContractSchemaState::Stale,
        "a referenced column that gained NULLs is Stale"
    );

    // Row 7: a referenced column became non-nullable -> NeedsReview. Losing
    // nullability only narrows what was already true.
    let obj_nn_base = table(&[("id", "bigint", false), ("amount", "numeric", true)]);
    let obj_nn = object_ref(&p, "nonnullable");
    let claim_nn = known_claim(
        &store,
        &obj_nn,
        &obj_nn_base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_nn = schema_tree_for(&[(
        "nonnullable",
        table(&[("id", "bigint", false), ("amount", "numeric", false)]),
    )]);
    assert_eq!(
        schema_state_for(&claim_nn, Some(&live_nn)),
        ContractSchemaState::NeedsReview,
        "a referenced column that lost nullability is NeedsReview, not Stale"
    );

    // Row 8: an unknown snapshot (pre-5a row) with a moved fingerprint ->
    // NeedsReview, never Current. The claim may be fine, but nothing can
    // prove it — the pre-5a behaviour, preserved exactly for pre-5a rows.
    let obj_unk = object_ref(&p, "unknown");
    let fp_unk = fingerprint_for(&base);
    let id_unk = confirm_candidate(
        &store,
        &obj_unk,
        &fp_unk,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let claim_unk = store.get_claim(&id_unk).await.unwrap().unwrap();
    assert!(
        claim_unk
            .referenced_columns
            .iter()
            .all(|c| c.data_type.is_empty()),
        "confirm_candidate writes name-only (unknown) snapshots"
    );
    let live_unk = schema_tree_for(&[(
        "unknown",
        table(&[
            ("id", "bigint", false),
            ("amount", "double precision", false),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim_unk, Some(&live_unk)),
        ContractSchemaState::NeedsReview,
        "an unknown snapshot plus a moved fingerprint is NeedsReview, not Current"
    );

    // Row 9: the object absent from live schema -> Stale.
    let obj_absent = object_ref(&p, "absent");
    let claim_absent = known_claim(
        &store,
        &obj_absent,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live_absent = schema_tree_for(&[]);
    assert_eq!(
        schema_state_for(&claim_absent, Some(&live_absent)),
        ContractSchemaState::Stale
    );

    // Row 10: no live schema -> LiveSchemaUnavailable.
    assert_eq!(
        schema_state_for(&claim_cur, None),
        ContractSchemaState::LiveSchemaUnavailable
    );

    // Row 11: an older fingerprint version -> NeedsReview even if everything
    // else matches. The version gate fires first, so the state is NeedsReview
    // regardless of the live table.
    let old = SchemaFingerprint::from_parts(2, claim_cur.schema_fingerprint.as_str()).unwrap();
    let claim_old = StoredClaim {
        schema_fingerprint: old,
        ..claim_cur.clone()
    };
    assert_eq!(
        schema_state_for(&claim_old, Some(&live_cur)),
        ContractSchemaState::NeedsReview,
        "an older fingerprint version is NeedsReview, never Current"
    );

    // Row 12: a claim with no referenced columns (a table description) and a
    // moved fingerprint -> NeedsReview, not Stale: nothing it depends on can
    // have broken.
    let obj_desc = object_ref(&p, "desc");
    let claim_desc = known_claim(
        &store,
        &obj_desc,
        &base,
        ClaimPayload::table_description("the orders table").unwrap(),
    )
    .await;
    assert!(
        claim_desc.referenced_columns.is_empty(),
        "a table description has no referenced columns"
    );
    let live_desc = schema_tree_for(&[(
        "desc",
        table(&[
            ("id", "bigint", false),
            ("amount", "numeric", false),
            ("note", "text", true),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim_desc, Some(&live_desc)),
        ContractSchemaState::NeedsReview,
        "a table-level claim whose columns all survive an unrelated change is NeedsReview"
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
    let shown = show(&store, &obj, None, RetrievalPolicy::ForHumanReview)
        .await
        .unwrap();
    assert!(shown.is_none(), "forgotten-only contract should show None");

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
async fn seed_computed_stale(store: &SqliteStateStore, object: &DatabaseObjectRef) -> StoredClaim {
    let full = table(&[("id", "bigint", false), ("created_at", "timestamp", false)]);
    let claim = known_claim(
        store,
        object,
        &full,
        ClaimPayload::default_time_column("created_at").unwrap(),
    )
    .await;
    // Live schema drops `created_at`; the claim's referenced column is gone.
    let drifted = schema_tree_for(&[(object.object(), table(&[("id", "bigint", false)]))]);
    store
        .upsert_schema(object.profile().as_str(), &drifted)
        .await
        .unwrap();
    claim
}

#[tokio::test]
async fn show_keeps_a_stale_contract_with_state_and_claims_for_a_human() {
    let root = temp_root("show_stale_human");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;
    let p = profile_a();
    let obj = object_ref(&p, "orders");
    let claim = seed_computed_stale(&store, &obj).await;

    // The human-review path: `show` keeps the stale contract, its `Stale`
    // state, and the claim itself so a reviewer can act on it. The model path
    // (`ForModel`) is what drops; `contracts show` is `ForHumanReview`.
    let shown = show(
        &store,
        &obj,
        Some(&schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForHumanReview,
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
        ids.contains(&claim.id),
        "the stale claim is kept for a human reviewer, got {ids:?}"
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
    seed_computed_stale(&store, &obj).await;

    // The model-facing path: `contract_read` uses `ForModel`. The object is
    // still named and reported `Stale`, but the gone-column claim is not handed
    // to the model as a current fact.
    let shown = show(
        &store,
        &obj,
        Some(&schema_tree_for(&[(
            "orders",
            table(&[("id", "bigint", false)]),
        )])),
        RetrievalPolicy::ForModel,
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
    seed_computed_stale(&store, &obj).await;

    // The model-facing recall path drops the computed-stale contract and
    // counts it in `excluded_by_schema`, so a user can see why a fact they
    // remembered stopped appearing — the exclusion is not silent.
    let drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), drifted)],
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
// Phase 5d: reconciliation writes — persisting the 5b verdict.
//
// `reconcile` examines only Candidate/Confirmed claims, computes the 5b state
// against a live schema per profile, and persists Stale (and only Stale) via
// the existing `mark_stale`. A profile with no live schema is skipped, never
// marked: a network blip must not destroy a user's accumulated knowledge. See
// .claude/specs/spec-5d-reconciliation-writes.md.
// ---------------------------------------------------------------------------

use super::reconcile;

/// A confirmed claim proposed against `live`, carrying a *known* per-column
/// snapshot (observed type + nullability) so the 5b rule can compute Stale or
/// NeedsReview rather than the unknown-snapshot fallback. Mirrors `known_claim`
/// in the validity matrix, lifted to module scope so every 5d test shares it.
async fn known_claim(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    live: &Table,
    payload: ClaimPayload,
) -> StoredClaim {
    let fp = SchemaFingerprint::of_table(DatabaseObjectKind::Table, live);
    let referenced = payload.referenced_column_snapshots(live);
    let req = ProposeClaim {
        object: object.clone(),
        fingerprint: fp,
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: referenced,
    };
    let id = match store.propose_claim(req).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    store.get_claim(&id).await.unwrap().unwrap()
}

/// The number of events recorded for `id` — the lever the "no new event"
/// assertions pull, since a reconcile that writes nothing must append nothing.
async fn event_count(store: &SqliteStateStore, id: &ClaimId) -> usize {
    store.claim_events(id, 1000).await.unwrap().len()
}

#[tokio::test]
async fn reconcile_marks_a_removed_column_stale_and_appends_marked_stale() {
    let root = temp_root("reconcile_removed");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;

    // Live schema drops `amount` -> the 5b rule reads Stale.
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(outcome.marked_stale, 1);
    assert!(outcome.examined >= 1);
    assert_eq!(outcome.skipped_unavailable, 0);
    assert!(!outcome.truncated);

    let after = store.get_claim(&claim.id).await.unwrap().unwrap();
    assert_eq!(after.status, ClaimStatus::Stale);

    // The transition is audited: a MarkedStale event from Confirmed -> Stale.
    let events = store.claim_events(&claim.id, 100).await.unwrap();
    let marked = events
        .iter()
        .find(|e| e.kind == ContractEventKind::MarkedStale);
    assert!(
        marked.is_some(),
        "expected a MarkedStale event, got {events:?}"
    );
    assert_eq!(marked.unwrap().from_status, Some(ClaimStatus::Confirmed));
    assert_eq!(marked.unwrap().to_status, Some(ClaimStatus::Stale));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_leaves_a_current_claim_untouched() {
    let root = temp_root("reconcile_current");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let before_status = claim.status;
    let before_updated = claim.updated_unix_ms;
    let before_events = event_count(&store, &claim.id).await;

    // The live schema is unchanged -> Current. Nothing is written.
    let live = schema_tree_for(&[("orders", base.clone())]);
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(
        outcome.marked_stale, 0,
        "a current claim must not be marked"
    );

    let after = store.get_claim(&claim.id).await.unwrap().unwrap();
    assert_eq!(after.status, before_status);
    assert_eq!(
        after.updated_unix_ms, before_updated,
        "a current claim's updated stamp must not move"
    );
    assert_eq!(
        event_count(&store, &claim.id).await,
        before_events,
        "a current claim must gain no new event"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn needs_review_is_computed_at_read_not_persisted() {
    let root = temp_root("reconcile_needs_review");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let before_events = event_count(&store, &claim.id).await;

    // An unrelated column is added: the fingerprint moves but nothing the claim
    // depends on broke, so the 5b rule computes NeedsReview. Reconcile must
    // NOT write it — NeedsReview is a read-time opinion, never persisted.
    let live = schema_tree_for(&[(
        "orders",
        table(&[
            ("id", "bigint", false),
            ("amount", "numeric", false),
            ("note", "text", true),
        ]),
    )]);
    assert_eq!(
        schema_state_for(&claim, Some(&live)),
        ContractSchemaState::NeedsReview,
        "fixture must compute NeedsReview for this test to mean anything"
    );
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(outcome.marked_stale, 0);

    let after = store.get_claim(&claim.id).await.unwrap().unwrap();
    assert_eq!(
        after.status,
        ClaimStatus::Confirmed,
        "NeedsReview must not be persisted as a status"
    );
    assert_eq!(
        event_count(&store, &claim.id).await,
        before_events,
        "a NeedsReview claim must gain no event"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_is_idempotent_a_second_pass_marks_nothing() {
    let root = temp_root("reconcile_idempotent");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);

    let first = reconcile(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live.clone())],
    )
    .await
    .unwrap();
    assert_eq!(first.marked_stale, 1);
    let events_after_first = event_count(&store, &claim.id).await;

    // The claim is now Stale, and Stale is not examined — so the second pass
    // marks nothing and appends nothing. `mark_stale` on an already-stale claim
    // is a Conflict, which a naive re-run would hit; reconciliation must skip it.
    let second = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(second.marked_stale, 0, "second pass must not re-mark");
    assert_eq!(
        event_count(&store, &claim.id).await,
        events_after_first,
        "second pass must append no events"
    );
    let after = store.get_claim(&claim.id).await.unwrap().unwrap();
    assert_eq!(after.status, ClaimStatus::Stale);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_skips_a_profile_with_no_live_schema_and_counts_it() {
    let root = temp_root("reconcile_unavailable");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    let before_events = event_count(&store, &claim.id).await;

    // The profile is asked for but no live schema is supplied for it: we could
    // not look, so we must not mark. The claims are counted as skipped, not
    // examined, and never marked.
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[])
        .await
        .unwrap();
    assert_eq!(outcome.marked_stale, 0);
    assert_eq!(
        outcome.skipped_unavailable, 1,
        "the skipped claim is counted"
    );
    assert_eq!(outcome.examined, 0);

    let after = store.get_claim(&claim.id).await.unwrap().unwrap();
    assert_eq!(after.status, ClaimStatus::Confirmed);
    assert_eq!(
        event_count(&store, &claim.id).await,
        before_events,
        "a skipped claim must gain no event"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_does_not_examine_rejected_forgotten_or_already_stale_claims() {
    let root = temp_root("reconcile_membership");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let fp = fingerprint_for(&base);

    // A confirmed claim that will drift to Stale (the only one examined).
    let confirmed = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;
    // A rejected claim: propose a candidate then reject it.
    let rejected = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("rj").unwrap(),
        &[],
    )
    .await;
    store.reject_claim(&rejected).await.unwrap();
    // A forgotten claim.
    let forgotten = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("fg").unwrap(),
        &[],
    )
    .await;
    store
        .forget_claim(&forgotten, ForgetReason::Obsolete)
        .await
        .unwrap();
    // An already-stale claim: propose a candidate then mark it stale directly.
    let prestale = propose_candidate(
        &store,
        &obj,
        &fp,
        ClaimPayload::table_alias("ps").unwrap(),
        &[],
    )
    .await;
    store.mark_stale(&prestale).await.unwrap();

    let rej_events = event_count(&store, &rejected).await;
    let fg_events = event_count(&store, &forgotten).await;
    let ps_events = event_count(&store, &prestale).await;

    let live = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(
        outcome.marked_stale, 1,
        "only the confirmed claim is marked"
    );
    assert_eq!(outcome.examined, 1, "only the confirmed claim is examined");

    // The three non-examined claims keep their statuses and gain no events.
    assert_eq!(
        store.get_claim(&rejected).await.unwrap().unwrap().status,
        ClaimStatus::Rejected
    );
    assert_eq!(
        store.get_claim(&forgotten).await.unwrap().unwrap().status,
        ClaimStatus::Forgotten
    );
    assert_eq!(
        store.get_claim(&prestale).await.unwrap().unwrap().status,
        ClaimStatus::Stale
    );
    assert_eq!(event_count(&store, &rejected).await, rej_events);
    assert_eq!(event_count(&store, &forgotten).await, fg_events);
    assert_eq!(event_count(&store, &prestale).await, ps_events);
    assert_eq!(
        store
            .get_claim(&confirmed.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ClaimStatus::Stale
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn a_stale_marked_claim_leaves_recall_and_enters_the_review_queue() {
    let root = temp_root("reconcile_recall_queue");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let claim = known_claim(
        &store,
        &obj,
        &base,
        ClaimPayload::column_description("amount", "how much").unwrap(),
    )
    .await;

    // Before reconcile the confirmed claim is recallable.
    let live_current = schema_tree_for(&[("orders", base.clone())]);
    let before = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), live_current.clone())],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        before.contracts.len(),
        1,
        "confirmed claim should be recallable"
    );

    // Reconcile against a schema that dropped `amount` -> Stale, persisted.
    let live_drifted = schema_tree_for(&[("orders", table(&[("id", "bigint", false)]))]);
    let outcome = reconcile(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_drifted.clone())],
    )
    .await
    .unwrap();
    assert_eq!(outcome.marked_stale, 1);

    // A stale claim is not recallable, so it disappears from recall.
    let after = recall(
        &store,
        recall_request(
            std::slice::from_ref(&p),
            &[(p.clone(), live_drifted.clone())],
            &["orders".to_string()],
            true,
            RecallBounds::defaults(),
        ),
    )
    .await;
    assert_eq!(
        after.contracts.len(),
        0,
        "a stale claim must not be recalled"
    );

    // And it surfaces in the review queue — a stale claim is waiting for a
    // human to decide its fate, which is the point of persisting Stale.
    let queued = review_queue(
        &store,
        std::slice::from_ref(&p),
        &[(p.clone(), live_drifted)],
        200,
    )
    .await
    .unwrap();
    let ids: Vec<ClaimId> = queued.iter().map(|q| q.claim.id.clone()).collect();
    assert!(
        ids.contains(&claim.id),
        "a stale-marked claim should appear in the review queue, got {ids:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_binds_at_1000_claims_and_reports_truncation() {
    let root = temp_root("reconcile_bound");
    let db = root.join("state.sqlite3");
    let store = store_at(&db).await;

    let p = profile_a();
    let fp = fingerprint_for(&table(&[
        ("id", "bigint", false),
        ("amount", "numeric", false),
    ]));

    // More than 1000 examine claims: 128 per object across 8 objects = 1024,
    // each with a distinct alias so they do not dedup. The per-object cap is
    // MAX_CLAIMS_PER_OBJECT (128), so the work spreads across objects.
    let mut ids: Vec<ClaimId> = Vec::new();
    for obj_idx in 0..8u32 {
        let obj = object_ref(&p, &format!("t{obj_idx}"));
        for i in 0..128u32 {
            let id = confirm_candidate(
                &store,
                &obj,
                &fp,
                ClaimPayload::table_alias(format!("a{obj_idx}-{i}")).unwrap(),
            )
            .await;
            ids.push(id);
        }
    }
    assert_eq!(ids.len(), 1024);

    // Every object is absent from the live schema -> each claim computes Stale.
    // Without the bound all 1024 would be marked; the bound caps examination at
    // 1000, so the remaining claims are left untouched and truncation reported.
    let live = SchemaTree::default();
    let outcome = reconcile(&store, std::slice::from_ref(&p), &[(p.clone(), live)])
        .await
        .unwrap();
    assert_eq!(
        outcome.examined, 1000,
        "the bound caps examination at 1000 claims"
    );
    assert_eq!(
        outcome.marked_stale, 1000,
        "each examined claim computed Stale"
    );
    assert!(
        outcome.truncated,
        "truncation must be reported when the bound fires"
    );

    // Exactly 1000 were marked; the 24 beyond the bound stayed Confirmed.
    let mut stale = 0;
    let mut confirmed = 0;
    for id in &ids {
        match store.get_claim(id).await.unwrap().unwrap().status {
            ClaimStatus::Stale => stale += 1,
            ClaimStatus::Confirmed => confirmed += 1,
            other => panic!("unexpected status {other:?} for {id}"),
        }
    }
    assert_eq!(stale, 1000);
    assert_eq!(confirmed, 24);

    let _ = fs::remove_dir_all(root);
}
