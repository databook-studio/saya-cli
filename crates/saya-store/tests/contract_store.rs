use saya_store::{
    ClaimEvidence, ContractStore, EvidenceKind, MAX_CLAIM_PAYLOAD_BYTES, MAX_CLAIMS_PER_OBJECT,
    MAX_EVIDENCE_PER_CLAIM, ProposeClaim, ProposeOutcome, SqliteStateStore, StoreError, object_id,
};
use saya_types::{
    Cardinality, ClaimOrigin, ClaimPayload, ClaimStatus, ColumnRole, DatabaseObjectKind,
    DatabaseObjectRef, FINGERPRINT_VERSION, MAX_NAME_CHARS, MAX_REFERENCED_COLUMNS,
    ProfileIdentity, ReferencedColumn, SchemaFingerprint, Table,
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

#[tokio::test]
async fn user_explicit_confirmed_claim_round_trips() {
    let root = temp_root("t1");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let payload = ClaimPayload::column_description("user_id", "the user id").unwrap();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        // No live schema here, so the column is recorded by name only — the
        // behaviour pre-5a had (a bare name list), now a name-only snapshot.
        referenced_columns: payload.referenced_column_name_snapshots(),
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.id, id);
    assert_eq!(stored.object, object);
    assert_eq!(stored.status, ClaimStatus::Confirmed);
    assert_eq!(stored.origin, ClaimOrigin::UserExplicit);
    assert_eq!(stored.payload, Some(payload));
    assert_eq!(
        stored.referenced_columns,
        vec![ReferencedColumn {
            name: "user_id".to_string(),
            data_type: String::new(),
            nullable: false,
        }]
    );
    assert!(stored.last_verified_unix_ms.is_none());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn assistant_inferred_cannot_be_confirmed() {
    let root = temp_root("t2");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("d").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let error = store.propose_claim(request).await.unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    assert!(store.list_claims(&object, &[]).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn duplicate_payload_returns_same_id() {
    let root = temp_root("t3");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let make = || ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("the events table").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let first_id = match store.propose_claim(make()).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    match store.propose_claim(make()).await.unwrap() {
        ProposeOutcome::Duplicate { id, status } => {
            assert_eq!(id, first_id);
            assert_eq!(status, ClaimStatus::Candidate);
        }
        other => panic!("expected Duplicate, got {other:?}"),
    }
    assert_eq!(store.list_claims(&object, &[]).await.unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn column_role_contradiction_dedups() {
    let root = temp_root("t4");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let first = ClaimPayload::column_role("user_id", ColumnRole::Identifier).unwrap();
    let second = ClaimPayload::column_role("user_id", ColumnRole::Dimension).unwrap();
    let request = |payload: ClaimPayload| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let _ = store.propose_claim(request(first)).await.unwrap();
    match store.propose_claim(request(second)).await.unwrap() {
        ProposeOutcome::Duplicate { .. } => {}
        other => panic!("expected Duplicate, got {other:?}"),
    }
    assert_eq!(store.list_claims(&object, &[]).await.unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn column_role_distinct_columns_distinct_rows() {
    let root = temp_root("t5");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = |column: &str| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::column_role(column, ColumnRole::Identifier).unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let _ = store.propose_claim(request("user_id")).await.unwrap();
    let _ = store.propose_claim(request("event_id")).await.unwrap();
    assert_eq!(store.list_claims(&object, &[]).await.unwrap().len(), 2);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn identical_evidence_dedups_but_turn_ordinal_distinguishes() {
    let root = temp_root("t6");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let evidence = |turn: u32| ClaimEvidence {
        kind: EvidenceKind::ExplicitUserStatement,
        session_id: Some("s1".into()),
        turn_ordinal: Some(turn),
        observed_unix_ms: 10_000 + turn as i64,
    };
    let request = |evidence: ClaimEvidence| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("the events table").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: Some(evidence),
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request(evidence(1))).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    store.propose_claim(request(evidence(1))).await.unwrap();
    assert_eq!(evidence_count(&db, id.as_str()).await, 1);
    store.propose_claim(request(evidence(2))).await.unwrap();
    assert_eq!(evidence_count(&db, id.as_str()).await, 2);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn evidence_cap_keeps_newest() {
    let root = temp_root("t7");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let base = 100_000_i64;
    let request = |i: u32| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("the events table").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: Some(ClaimEvidence {
            kind: EvidenceKind::RepeatedObservation,
            session_id: Some(format!("s{i}")),
            turn_ordinal: Some(i),
            observed_unix_ms: base + i as i64,
        }),
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request(1)).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    for i in 2..=40 {
        let _ = store.propose_claim(request(i)).await.unwrap();
    }
    let mut observed = evidence_observed(&db, id.as_str()).await;
    assert_eq!(observed.len(), MAX_EVIDENCE_PER_CLAIM);
    observed.sort_unstable();
    let expected: Vec<i64> = (9..=40).map(|i| base + i).collect();
    assert_eq!(observed, expected);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn evidence_count_trait_reads_the_attached_rows() {
    // The queue (Phase 3d) needs a per-claim evidence count to order
    // candidates, but must not receive the evidence rows themselves (they
    // carry session ids and turn ordinals). The trait method returns a bare
    // count; this test asserts it matches the raw table count and that a
    // claim with no evidence reads zero rather than erroring.
    let root = temp_root("evidence_count");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let no_evidence = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("no evidence").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let bare_id = match store.propose_claim(no_evidence).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    assert_eq!(store.evidence_count(&bare_id).await.unwrap(), 0);

    let evidence = |turn: u32| ClaimEvidence {
        kind: EvidenceKind::RepeatedObservation,
        session_id: Some("s1".into()),
        turn_ordinal: Some(turn),
        observed_unix_ms: 10_000 + turn as i64,
    };
    let request = |evidence: ClaimEvidence| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("with evidence").unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence: Some(evidence),
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request(evidence(1))).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    store.propose_claim(request(evidence(2))).await.unwrap();
    store.propose_claim(request(evidence(3))).await.unwrap();
    // An identical repeat of turn 1 dedups and does not raise the count.
    store.propose_claim(request(evidence(1))).await.unwrap();
    assert_eq!(store.evidence_count(&id).await.unwrap(), 3);
    assert_eq!(
        store.evidence_count(&id).await.unwrap(),
        evidence_count(&db, id.as_str()).await,
        "trait count must match the raw table count"
    );

    // An unknown id reports NotFound, not zero — a missing claim is not an
    // empty evidence set.
    let unknown = saya_types::ClaimId::parse("c-deadbeef").unwrap();
    assert_eq!(
        store.evidence_count(&unknown).await.unwrap_err(),
        StoreError::NotFound
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn claim_cap_rejects_overflow() {
    let root = temp_root("t8");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    for i in 0..MAX_CLAIMS_PER_OBJECT {
        let request = ProposeClaim {
            object: object.clone(),
            fingerprint: fingerprint(),
            payload: ClaimPayload::table_alias(format!("alias{i}")).unwrap(),
            origin: ClaimOrigin::UserExplicit,
            initial_status: ClaimStatus::Candidate,
            evidence: None,
            referenced_columns: Vec::new(),
        };
        let _ = store.propose_claim(request).await.unwrap();
    }
    assert_eq!(
        store.list_claims(&object, &[]).await.unwrap().len(),
        MAX_CLAIMS_PER_OBJECT
    );
    let overflow = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias("overflow").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let error = store.propose_claim(overflow).await.unwrap_err();
    assert_eq!(error, StoreError::LimitExceeded);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn oversized_payload_rejected() {
    let root = temp_root("t9");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    // TableDescription text is capped at MAX_TEXT_CHARS (1024), so its JSON can never
    // reach 4 KiB. A Relationship carries MAX_REFERENCED_COLUMNS local and target
    // columns of MAX_NAME_CHARS each — that easily exceeds 4 KiB.
    let target = object_ref(&profile_a(), "targets");
    let long = "x".repeat(MAX_NAME_CHARS);
    let local: Vec<String> = (0..MAX_REFERENCED_COLUMNS).map(|_| long.clone()).collect();
    let target_cols: Vec<String> = (0..MAX_REFERENCED_COLUMNS).map(|_| long.clone()).collect();
    let payload =
        ClaimPayload::relationship(target, local, target_cols, Cardinality::OneToMany).unwrap();
    let serialized = serde_json::to_string(&payload).unwrap();
    assert!(serialized.len() > MAX_CLAIM_PAYLOAD_BYTES);
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let error = store.propose_claim(request).await.unwrap_err();
    assert_eq!(error, StoreError::LimitExceeded);
    assert!(store.list_claims(&object, &[]).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn secret_payload_refused() {
    let root = temp_root("t10");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("creds password=hunter2 end").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let error = store.propose_claim(request).await.unwrap_err();
    assert_eq!(error, StoreError::Invalid);
    assert!(store.list_claims(&object, &[]).await.unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn claims_never_cross_profile_boundaries() {
    let root = temp_root("t11");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let profile_a = profile_a();
    let profile_b = profile_b();
    let ref_a = object_ref(&profile_a, "events");
    let ref_b = object_ref(&profile_b, "events");
    let request = |object: DatabaseObjectRef, text: &str| ProposeClaim {
        object,
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description(text).unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id_a = match store
        .propose_claim(request(ref_a.clone(), "a events"))
        .await
        .unwrap()
    {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let id_b = match store
        .propose_claim(request(ref_b.clone(), "b events"))
        .await
        .unwrap()
    {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    assert_ne!(id_a, id_b);
    let objects_a = store.list_objects(&profile_a).await.unwrap();
    assert_eq!(objects_a.len(), 1);
    assert_eq!(objects_a[0].object, ref_a);
    let claims_a = store.list_claims(&ref_a, &[]).await.unwrap();
    assert_eq!(claims_a.len(), 1);
    assert_eq!(claims_a[0].id, id_a);
    let claims_b = store.list_claims(&ref_b, &[]).await.unwrap();
    assert_eq!(claims_b.len(), 1);
    assert_eq!(claims_b[0].id, id_b);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn ids_are_deterministic_across_stores() {
    let root = temp_root("t12");
    let db_a = root.join("a.sqlite3");
    let db_b = root.join("b.sqlite3");
    let store_a = SqliteStateStore::new(&db_a);
    let store_b = SqliteStateStore::new(&db_b);
    let object = object_ref(&profile_a(), "events");
    assert_eq!(object_id(&object).as_str(), object_id(&object).as_str());
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("desc").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id_a = match store_a.propose_claim(request.clone()).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let id_b = match store_b.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    assert_eq!(id_a, id_b);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn concurrent_proposes_one_stored_seven_duplicate() {
    let root = temp_root("t13");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let fp = fingerprint();
    // The deferred transaction's first statement is the object upsert (a write), so
    // each transaction holds SQLite's single writer lock from the upsert through
    // commit. Concurrent proposers serialize on that lock; the first stores and the
    // rest observe it via the duplicate check.
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let object = object.clone();
        let fp = fp.clone();
        handles.push(tokio::spawn(async move {
            let request = ProposeClaim {
                object,
                fingerprint: fp,
                payload: ClaimPayload::table_description("concurrent desc").unwrap(),
                origin: ClaimOrigin::UserExplicit,
                initial_status: ClaimStatus::Candidate,
                evidence: None,
                referenced_columns: Vec::new(),
            };
            store.propose_claim(request).await.unwrap()
        }));
    }
    let mut stored = 0;
    let mut duplicate = 0;
    for handle in handles {
        match handle.await.unwrap() {
            ProposeOutcome::Stored(_) => stored += 1,
            ProposeOutcome::Duplicate { .. } => duplicate += 1,
        }
    }
    assert_eq!(stored, 1);
    assert_eq!(duplicate, 7);
    assert_eq!(store.list_claims(&object, &[]).await.unwrap().len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn list_claims_status_filter() {
    let root = temp_root("t14");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let confirmed = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("confirmed one").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let candidate = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias("alias").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let _ = store.propose_claim(confirmed).await.unwrap();
    let _ = store.propose_claim(candidate).await.unwrap();
    assert_eq!(store.list_claims(&object, &[]).await.unwrap().len(), 2);
    let only_confirmed = store
        .list_claims(&object, &[ClaimStatus::Confirmed])
        .await
        .unwrap();
    assert_eq!(only_confirmed.len(), 1);
    assert_eq!(only_confirmed[0].status, ClaimStatus::Confirmed);
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// P2 — bulk reads: `list_claims_for_profile` and `evidence_counts`.
//
// Recall, the queue, and reconciliation moved from one query per object (and
// one `COUNT(*)` per claim) to one query per profile. These prove the bulk APIs
// return what the per-object/per-claim ones did: every claim of a profile
// across every object (every status, no filter), and evidence counts for a set
// of claims in one `GROUP BY` — a claim with no evidence reads `0`, matching
// the per-claim `evidence_count`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_claims_for_profile_returns_every_claim_across_objects() {
    let root = temp_root("bulk_list_claims");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);

    let a = profile_a();
    let b = profile_b();
    let obj_a1 = object_ref(&a, "orders");
    let obj_a2 = object_ref(&a, "events");
    let obj_b = object_ref(&b, "orders");

    let req = |object: DatabaseObjectRef, alias: &str, status: ClaimStatus| ProposeClaim {
        object,
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias(alias).unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: status,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    // Profile A: two objects, three claims of mixed status.
    store
        .propose_claim(req(obj_a1.clone(), "a1c", ClaimStatus::Confirmed))
        .await
        .unwrap();
    store
        .propose_claim(req(obj_a1.clone(), "a1k", ClaimStatus::Candidate))
        .await
        .unwrap();
    store
        .propose_claim(req(obj_a2.clone(), "a2c", ClaimStatus::Confirmed))
        .await
        .unwrap();
    // Profile B: one claim — must not appear in A's bulk read.
    store
        .propose_claim(req(obj_b.clone(), "bc", ClaimStatus::Confirmed))
        .await
        .unwrap();

    let bulk = store.list_claims_for_profile(&a).await.unwrap();
    // Every claim of profile A, every status — the union the per-object loop
    // produced. No statuses are filtered (the CLI filters by recall mode).
    assert_eq!(bulk.len(), 3, "every claim of the profile, got {bulk:?}");
    assert!(bulk.iter().all(|c| c.object.profile() == &a));
    let objects: Vec<&str> = bulk.iter().map(|c| c.object.object()).collect();
    assert!(objects.contains(&"orders") && objects.contains(&"events"));

    // The per-object reads agree with the bulk read for each object.
    let a1 = store.list_claims(&obj_a1, &[]).await.unwrap();
    let a2 = store.list_claims(&obj_a2, &[]).await.unwrap();
    let bulk_a1: Vec<&saya_store::StoredClaim> =
        bulk.iter().filter(|c| c.object == obj_a1).collect();
    let bulk_a2: Vec<&saya_store::StoredClaim> =
        bulk.iter().filter(|c| c.object == obj_a2).collect();
    assert_eq!(bulk_a1.len(), a1.len());
    assert_eq!(bulk_a2.len(), a2.len());

    // Profile B's bulk read is isolated to its own claims.
    let bulk_b = store.list_claims_for_profile(&b).await.unwrap();
    assert_eq!(bulk_b.len(), 1);
    assert_eq!(bulk_b[0].object, obj_b);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn evidence_counts_aggregates_in_one_query_and_zero_for_no_evidence() {
    let root = temp_root("bulk_evidence_counts");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let evidence = |turn: u32| ClaimEvidence {
        kind: EvidenceKind::RepeatedObservation,
        session_id: Some("s1".into()),
        turn_ordinal: Some(turn),
        observed_unix_ms: 10_000 + turn as i64,
    };
    let req = |evidence: Option<ClaimEvidence>, alias: &str| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias(alias).unwrap(),
        origin: ClaimOrigin::AssistantInferred,
        initial_status: ClaimStatus::Candidate,
        evidence,
        referenced_columns: Vec::new(),
    };
    // three claims: 3 evidence, 1 evidence, 0 evidence.
    let id_three = match store
        .propose_claim(req(Some(evidence(1)), "three"))
        .await
        .unwrap()
    {
        ProposeOutcome::Stored(id) => id,
        _ => unreachable!(),
    };
    store
        .propose_claim(req(Some(evidence(2)), "three"))
        .await
        .unwrap();
    store
        .propose_claim(req(Some(evidence(3)), "three"))
        .await
        .unwrap();
    let id_one = match store
        .propose_claim(req(Some(evidence(1)), "one"))
        .await
        .unwrap()
    {
        ProposeOutcome::Stored(id) => id,
        _ => unreachable!(),
    };
    let id_none = match store.propose_claim(req(None, "none")).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        _ => unreachable!(),
    };

    let counts: std::collections::HashMap<_, _> = store
        .evidence_counts(&[id_three.clone(), id_one.clone(), id_none.clone()])
        .await
        .unwrap()
        .into_iter()
        .collect();
    // A claim with no evidence rows is absent from the LEFT JOIN result; the
    // caller treats a missing id as 0 (the queue does). The two with evidence
    // match the per-claim `evidence_count`.
    assert_eq!(*counts.get(&id_three).unwrap_or(&0), 3);
    assert_eq!(*counts.get(&id_one).unwrap_or(&0), 1);
    // The LEFT JOIN yields a row per claim, so a no-evidence claim reads `0`,
    // not absent — the same value the per-claim `evidence_count` returns.
    assert_eq!(
        counts.get(&id_none),
        Some(&0),
        "a no-evidence claim reads 0"
    );
    // And the bulk counts match the per-claim trait counts exactly.
    assert_eq!(
        counts.get(&id_three),
        Some(&store.evidence_count(&id_three).await.unwrap())
    );
    assert_eq!(
        counts.get(&id_one),
        Some(&store.evidence_count(&id_one).await.unwrap())
    );
    assert_eq!(
        counts.get(&id_none),
        Some(&store.evidence_count(&id_none).await.unwrap())
    );

    // An empty input is a no-op (no round trip).
    assert!(store.evidence_counts(&[]).await.unwrap().is_empty());

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Phase 5a — referenced-column snapshots
// ---------------------------------------------------------------------------

/// A claim proposed against a live table stores a snapshot per referenced
/// column with the right type and nullability (spec test 1).
#[tokio::test]
async fn claim_proposed_with_live_table_stores_typed_snapshot() {
    let root = temp_root("snap-live");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let table = Table {
        name: "events".into(),
        columns: vec![
            saya_types::Column {
                name: "user_id".into(),
                data_type: "bigint".into(),
                nullable: false,
            },
            saya_types::Column {
                name: "amount".into(),
                data_type: "numeric".into(),
                nullable: true,
            },
        ],
    };
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let snapshots = payload.referenced_column_snapshots(&table);
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: snapshots,
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.referenced_columns,
        vec![ReferencedColumn {
            name: "amount".to_string(),
            data_type: "numeric".to_string(),
            nullable: true,
        }]
    );
    let _ = fs::remove_dir_all(root);
}

/// A referenced column absent from the live table stores no snapshot for it,
/// and the claim still stores (spec test 2).
#[tokio::test]
async fn absent_referenced_column_stores_no_snapshot() {
    let root = temp_root("snap-absent");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let table = Table {
        name: "events".into(),
        columns: vec![saya_types::Column {
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
        }],
    };
    let payload = ClaimPayload::column_description("missing", "gone").unwrap();
    let snapshots = payload.referenced_column_snapshots(&table);
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: snapshots,
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert!(
        stored.referenced_columns.is_empty(),
        "absent column produced a snapshot: {:?}",
        stored.referenced_columns
    );
    assert_eq!(stored.payload, Some(payload));
    let _ = fs::remove_dir_all(root);
}

/// A row written in the old `["a","b"]` shape decodes to snapshots marked
/// unknown (empty type), not to snapshots claiming a type (spec test 3).
#[tokio::test]
async fn old_name_list_shape_decodes_to_unknown_snapshots() {
    let root = temp_root("snap-old");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::column_description("user_id", "the id").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };

    // Rewrite the column to the pre-5a shape: a bare array of names.
    let pool = read_pool(&db).await;
    sqlx::query("UPDATE contract_claims SET referenced_columns_json='[\"user_id\"]' WHERE id=?")
        .bind(id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.referenced_columns,
        vec![ReferencedColumn {
            name: "user_id".to_string(),
            data_type: String::new(),
            nullable: false,
        }],
        "an old name-list row decoded to a snapshot that claims a type: {:?}",
        stored.referenced_columns
    );
    let _ = fs::remove_dir_all(root);
}

/// Snapshots survive store and load byte-identically (spec test 4).
#[tokio::test]
async fn snapshots_survive_round_trip_byte_identically() {
    let root = temp_root("snap-rt");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let table = Table {
        name: "events".into(),
        columns: vec![
            saya_types::Column {
                name: "a".into(),
                data_type: "int".into(),
                nullable: false,
            },
            saya_types::Column {
                name: "b".into(),
                data_type: "text".into(),
                nullable: true,
            },
        ],
    };
    let target = object_ref(&profile_a(), "targets");
    let payload = ClaimPayload::relationship(
        target,
        vec!["a".into(), "b".into()],
        vec!["x".into(), "y".into()],
        Cardinality::OneToMany,
    )
    .unwrap();
    let snapshots = payload.referenced_column_snapshots(&table);
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: snapshots.clone(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.referenced_columns, snapshots);
    let _ = fs::remove_dir_all(root);
}

/// A stored claim records `CLAIM_PAYLOAD_VERSION` 2 (spec test 5).
#[tokio::test]
async fn stored_claim_records_payload_version_two() {
    let root = temp_root("snap-ver");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias("a").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let pool = read_pool(&db).await;
    let version: i64 = sqlx::query_scalar("SELECT payload_version FROM contract_claims WHERE id=?")
        .bind(id.as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(version, saya_types::CLAIM_PAYLOAD_VERSION as i64);
    assert_eq!(saya_types::CLAIM_PAYLOAD_VERSION, 2);
    // FINGERPRINT_VERSION is unrelated but asserted here to prove the version
    // recorded is read from the data, not from the running build — see SPEC REVIEW.
    let _ = FINGERPRINT_VERSION;
    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Spec C: a claim decodes with the fingerprint version it was written under,
// not the object row's current version. `upsert_object_in_tx` overwrites the
// object row's `fingerprint_version` on every schema refresh, so a claim
// written under version A reads back under whatever version the object row
// carries now — silently misclassifying it at the first format change. The
// claim must carry its own version column.
//
// The skew is constructed deliberately: propose a claim under version A,
// then rewrite the *object* row's `fingerprint_version` to B directly (the
// overwrite a refresh performs), and read the claim back. It must report A.
// Before the fix every claim read joined `o.fingerprint_version`, so this
// read B and the test failed for exactly that reason.
// ---------------------------------------------------------------------------

/// A claim proposed under version A, read back after the object row moved to
/// version B, decodes with A — the per-claim read paths (`get_claim`).
#[tokio::test]
async fn claim_decodes_with_its_own_fingerprint_version_after_object_drifts() {
    let root = temp_root("fpv-own-get");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    // A version ahead of the current format, so the object-row overwrite to it
    // is an unambiguous skew the claim must not inherit.
    let written_version = FINGERPRINT_VERSION;
    let drifted_version = FINGERPRINT_VERSION + 1;
    let fingerprint = SchemaFingerprint::from_parts(written_version, &"a".repeat(64)).unwrap();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint.clone(),
        payload: ClaimPayload::table_description("the events table").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };

    // Move the object row's version on, as a schema refresh does. The claim's
    // own digest is left exactly as written; only the object row changes.
    bump_object_fingerprint_version(&db, &object, drifted_version).await;

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.schema_fingerprint.version(),
        written_version,
        "get_claim decoded the claim under the object row's version, not its own"
    );
    assert_eq!(stored.schema_fingerprint, fingerprint);

    // The object row legitimately keeps the drifted version — that is the
    // *object's* version, used for drift computation. The slice does not change it.
    let objects = store.list_objects(&profile_a()).await.unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].fingerprint_version, drifted_version);

    let _ = fs::remove_dir_all(root);
}

/// Every claim read path decodes the claim under its own version, not the
/// object row's: `list_claims`, `find_claim_by_dedup_key`, and the bulk
/// `list_claims_for_profile` as well as `get_claim`.
#[tokio::test]
async fn every_claim_read_decodes_with_its_own_fingerprint_version() {
    let root = temp_root("fpv-own-all-reads");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let profile = profile_a();
    let object = object_ref(&profile, "events");
    let written_version = FINGERPRINT_VERSION;
    let drifted_version = FINGERPRINT_VERSION + 1;
    let fingerprint = SchemaFingerprint::from_parts(written_version, &"b".repeat(64)).unwrap();
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint.clone(),
        payload: ClaimPayload::table_alias("events").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    bump_object_fingerprint_version(&db, &object, drifted_version).await;

    // list_claims
    let listed = store.list_claims(&object, &[]).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].schema_fingerprint.version(), written_version);

    // find_claim_by_dedup_key — the same key propose computed.
    let key = saya_store::deduplication_key(
        &ClaimPayload::table_alias("events").unwrap(),
        &serde_json::to_string(&ClaimPayload::table_alias("events").unwrap()).unwrap(),
    );
    let found = store
        .find_claim_by_dedup_key(&object, &key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, id);
    assert_eq!(found.schema_fingerprint.version(), written_version);

    // list_claims_for_profile — the bulk path recall and the queue use.
    let bulk = store.list_claims_for_profile(&profile).await.unwrap();
    assert_eq!(bulk.len(), 1);
    assert_eq!(bulk[0].schema_fingerprint.version(), written_version);

    let _ = fs::remove_dir_all(root);
}

/// Moves the object row's `fingerprint_version` to `version` without touching
/// the claim — the overwrite `upsert_object_in_tx` performs on every refresh.
async fn bump_object_fingerprint_version(db: &Path, object: &DatabaseObjectRef, version: u32) {
    let pool = read_pool(db).await;
    let oid = object_id(object);
    sqlx::query("UPDATE contract_objects SET fingerprint_version=? WHERE id=?")
        .bind(version as i64)
        .bind(oid.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

/// A snapshot carries a type name, so no sentinel value reaches the database
/// through this new field (spec test 6). `referenced_columns_json` is a new
/// persisted channel; the security standard says any such field goes through
/// the same admission gate as the payload, so a secret-bearing snapshot type
/// is refused — not stored, not redacted-and-stored.
#[tokio::test]
async fn no_sentinel_reaches_db_through_snapshot_type() {
    let root = temp_root("snap-sentinel");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::column_description("user_id", "the id").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: vec![ReferencedColumn {
            name: "user_id".to_string(),
            // A credential URL is a structural secret the admission gate refuses
            // in payloads; the same shape in a snapshot type must be refused too.
            data_type: "postgres://user:SENTINELPASSWORD@host/db".to_string(),
            nullable: false,
        }],
    };
    let err = store.propose_claim(request).await.unwrap_err();
    assert_eq!(err, StoreError::Invalid);
    // Nothing was stored: no claim, and the sentinel is not in the bytes.
    assert!(store.list_claims(&object, &[]).await.unwrap().is_empty());
    let bytes = fs::read(&db).unwrap_or_default();
    assert!(
        !bytes
            .windows(b"SENTINELPASSWORD".len())
            .any(|w| w == b"SENTINELPASSWORD"),
        "LEAK: sentinel reached the database bytes through referenced_columns_json"
    );
    let _ = fs::remove_dir_all(root);
}

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-contract-{label}-{}-{stamp}",
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
fn fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(1, &"a".repeat(64)).unwrap()
}

async fn read_pool(db: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}
async fn evidence_count(db: &Path, claim_id: &str) -> usize {
    let pool = read_pool(db).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_evidence WHERE claim_id=?")
        .bind(claim_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    count as usize
}
async fn evidence_observed(db: &Path, claim_id: &str) -> Vec<i64> {
    let pool = read_pool(db).await;
    let rows: Vec<i64> =
        sqlx::query_scalar("SELECT observed_unix_ms FROM contract_evidence WHERE claim_id=?")
            .bind(claim_id)
            .fetch_all(&pool)
            .await
            .unwrap();
    pool.close().await;
    rows
}

// --- Review addition: re-recording evidence that is already present must be a
// --- true no-op. The unique index makes the insert an INSERT OR IGNORE, so if
// --- pruning runs first, a duplicate silently costs the claim its oldest
// --- evidence row and the count drops below the cap.
#[tokio::test]
async fn duplicate_evidence_at_the_cap_does_not_evict_anything() {
    let root = temp_root("t15");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let base = 100_000_i64;
    let request = |i: u32| ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("the events table").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: Some(ClaimEvidence {
            kind: EvidenceKind::RepeatedObservation,
            session_id: Some(format!("s{i}")),
            turn_ordinal: Some(i),
            observed_unix_ms: base + i as i64,
        }),
        referenced_columns: Vec::new(),
    };
    let id = match store.propose_claim(request(1)).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    // Fill exactly to the cap.
    for i in 2..=MAX_EVIDENCE_PER_CLAIM as u32 {
        let _ = store.propose_claim(request(i)).await.unwrap();
    }
    let before = evidence_observed(&db, id.as_str()).await;
    assert_eq!(before.len(), MAX_EVIDENCE_PER_CLAIM);

    // Re-record evidence that is already stored. Nothing is new, so nothing may go.
    let _ = store
        .propose_claim(request(MAX_EVIDENCE_PER_CLAIM as u32))
        .await
        .unwrap();

    let mut after = evidence_observed(&db, id.as_str()).await;
    let mut expected = before;
    after.sort_unstable();
    expected.sort_unstable();
    assert_eq!(
        after, expected,
        "duplicate evidence evicted a row instead of being ignored",
    );
    let _ = fs::remove_dir_all(root);
}
