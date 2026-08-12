use saya_store::{
    ClaimEvidence, ContractStore, EvidenceKind, MAX_CLAIM_PAYLOAD_BYTES, MAX_CLAIMS_PER_OBJECT,
    MAX_EVIDENCE_PER_CLAIM, ProposeClaim, ProposeOutcome, SqliteStateStore, StoreError, object_id,
};
use saya_types::{
    Cardinality, ClaimOrigin, ClaimPayload, ClaimStatus, ColumnRole, DatabaseObjectKind,
    DatabaseObjectRef, MAX_NAME_CHARS, MAX_REFERENCED_COLUMNS, ProfileIdentity, SchemaFingerprint,
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
    assert_eq!(stored.referenced_columns, vec!["user_id".to_string()]);
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
    };
    let candidate = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_alias("alias").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
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
