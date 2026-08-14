use saya_store::{
    ClaimEvidence, ContractEventKind, ContractStore, EvidenceKind, ForgetReason,
    MAX_CLAIM_PAYLOAD_BYTES, ProposeClaim, ProposeOutcome, SqliteStateStore, StoreError,
};
use saya_types::{
    Cardinality, ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, DatabaseObjectKind,
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-lifecycle-{label}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn profile_a() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
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

async fn event_count(db: &Path, claim_id: &str) -> usize {
    let pool = read_pool(db).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contract_events WHERE claim_id=?")
        .bind(claim_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    count as usize
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

async fn propose_candidate(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
    origin: ClaimOrigin,
) -> ClaimId {
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload,
        origin,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

async fn propose_confirmed(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    payload: ClaimPayload,
) -> ClaimId {
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 1: Candidate → confirm → Confirmed
// ---------------------------------------------------------------------------
#[tokio::test]
async fn candidate_to_confirmed() {
    let root = temp_root("t1");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("the events table").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    let confirmed = store.confirm_claim(&id).await.unwrap();
    assert_eq!(confirmed.status, ClaimStatus::Confirmed);
    assert!(confirmed.last_verified_unix_ms.is_some());

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.status, ClaimStatus::Confirmed);
    assert!(stored.last_verified_unix_ms.is_some());

    let events = store.claim_events(&id, 10).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, ContractEventKind::Proposed);
    assert_eq!(events[0].to_status, Some(ClaimStatus::Candidate));
    assert!(events[0].from_status.is_none());
    assert_eq!(events[1].kind, ContractEventKind::Confirmed);
    assert_eq!(events[1].from_status, Some(ClaimStatus::Candidate));
    assert_eq!(events[1].to_status, Some(ClaimStatus::Confirmed));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: Confirm twice → Conflict
// ---------------------------------------------------------------------------
#[tokio::test]
async fn confirm_twice_conflict() {
    let root = temp_root("t2");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("the events table").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    store.confirm_claim(&id).await.unwrap();
    let err = store.confirm_claim(&id).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: Reject from Candidate works; reject from Confirmed is Conflict
// ---------------------------------------------------------------------------
#[tokio::test]
async fn reject_from_candidate_reject_from_confirmed() {
    let root = temp_root("t3");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("reject me").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    let rejected = store.reject_claim(&id).await.unwrap();
    assert_eq!(rejected.status, ClaimStatus::Rejected);

    let id2 = propose_confirmed(&store, &object, ClaimPayload::table_alias("keep").unwrap()).await;

    let err = store.reject_claim(&id2).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 4: Edit replaces payload, sets Confirmed, keeps origin & created_unix_ms
// ---------------------------------------------------------------------------
#[tokio::test]
async fn edit_replaces_payload() {
    let root = temp_root("t4");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("original text").unwrap(),
        ClaimOrigin::AssistantInferred,
    )
    .await;

    let original = store.get_claim(&id).await.unwrap().unwrap();
    let created = original.created_unix_ms;

    let new_payload = ClaimPayload::table_description("edited text").unwrap();
    let edited = store.edit_claim(&id, new_payload.clone()).await.unwrap();
    assert_eq!(edited.status, ClaimStatus::Confirmed);
    assert_eq!(edited.origin, ClaimOrigin::AssistantInferred);
    assert_eq!(edited.created_unix_ms, created);
    assert_eq!(edited.payload, Some(new_payload));

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.status, ClaimStatus::Confirmed);
    assert_eq!(
        stored.payload.as_ref().and_then(|p| {
            if let ClaimPayload::TableDescription { text, .. } = p {
                Some(text.as_str())
            } else {
                None
            }
        }),
        Some("edited text")
    );

    let events = store.claim_events(&id, 10).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, ContractEventKind::Edited);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5: Edit collision with another claim on the same object → Conflict
// ---------------------------------------------------------------------------
#[tokio::test]
async fn edit_collision_conflict() {
    let root = temp_root("t5");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id_a =
        propose_confirmed(&store, &object, ClaimPayload::table_alias("alpha").unwrap()).await;

    let id_b = propose_confirmed(&store, &object, ClaimPayload::table_alias("beta").unwrap()).await;

    let collision = ClaimPayload::table_alias("alpha").unwrap();
    let err = store.edit_claim(&id_b, collision).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    let a = store.get_claim(&id_a).await.unwrap().unwrap();
    let b = store.get_claim(&id_b).await.unwrap().unwrap();
    assert_eq!(a.payload, Some(ClaimPayload::table_alias("alpha").unwrap()));
    assert_eq!(b.payload, Some(ClaimPayload::table_alias("beta").unwrap()));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: Edit to oversized payload → LimitExceeded
// ---------------------------------------------------------------------------
#[tokio::test]
async fn edit_oversized_payload() {
    let root = temp_root("t6");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    let target = object_ref(&profile_a(), "targets");

    let id = propose_confirmed(&store, &object, ClaimPayload::table_alias("small").unwrap()).await;

    let long = "x".repeat(MAX_NAME_CHARS);
    let local: Vec<String> = (0..MAX_REFERENCED_COLUMNS).map(|_| long.clone()).collect();
    let target_cols: Vec<String> = (0..MAX_REFERENCED_COLUMNS).map(|_| long.clone()).collect();
    let oversized =
        ClaimPayload::relationship(target, local, target_cols, Cardinality::OneToMany).unwrap();
    let serialized = serde_json::to_string(&oversized).unwrap();
    assert!(serialized.len() > MAX_CLAIM_PAYLOAD_BYTES);

    let err = store.edit_claim(&id, oversized).await.unwrap_err();
    assert_eq!(err, StoreError::LimitExceeded);

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.payload,
        Some(ClaimPayload::table_alias("small").unwrap())
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 7: Edit to payload with secret → Invalid
// ---------------------------------------------------------------------------
#[tokio::test]
async fn edit_secret_payload() {
    let root = temp_root("t7");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_confirmed(&store, &object, ClaimPayload::table_alias("clean").unwrap()).await;

    let secret = ClaimPayload::table_description("password=hunter2 end").unwrap();
    let err = store.edit_claim(&id, secret).await.unwrap_err();
    assert_eq!(err, StoreError::Invalid);

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.payload,
        Some(ClaimPayload::table_alias("clean").unwrap())
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 8: Forget erases content
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forgetting_a_claim_erases_its_payload() {
    let root = temp_root("t8");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("will be forgotten").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    // Add some evidence via duplicate proposal
    let ev = ClaimEvidence {
        kind: EvidenceKind::ExplicitUserStatement,
        session_id: Some("s1".into()),
        turn_ordinal: Some(1),
        observed_unix_ms: 1_000,
    };
    let dup = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: ClaimPayload::table_description("will be forgotten").unwrap(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: Some(ev),
        referenced_columns: Vec::new(),
    };
    let _ = store.propose_claim(dup).await.unwrap();

    store
        .forget_claim(&id, ForgetReason::UserRequest)
        .await
        .unwrap();

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.payload, None);
    assert_eq!(stored.status, ClaimStatus::Forgotten);
    assert!(stored.referenced_columns.is_empty());
    assert_eq!(evidence_count(&db, id.as_str()).await, 0);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 9: Forget removes from recall
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forget_removes_from_recall() {
    let root = temp_root("t9");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let _other =
        propose_confirmed(&store, &object, ClaimPayload::table_alias("keep").unwrap()).await;
    let forgotten = propose_confirmed(
        &store,
        &object,
        ClaimPayload::table_alias("remove").unwrap(),
    )
    .await;
    store
        .forget_claim(&forgotten, ForgetReason::Obsolete)
        .await
        .unwrap();

    let confirmed_only = store
        .list_claims(&object, &[ClaimStatus::Confirmed])
        .await
        .unwrap();
    assert_eq!(confirmed_only.len(), 1);
    assert_ne!(confirmed_only[0].id, forgotten);

    let recallable: Vec<ClaimStatus> = [
        ClaimStatus::Candidate,
        ClaimStatus::Confirmed,
        ClaimStatus::Rejected,
        ClaimStatus::Stale,
        ClaimStatus::Contradicted,
        ClaimStatus::Forgotten,
    ]
    .into_iter()
    .filter(|s| s.is_recallable())
    .collect();
    let filtered = store.list_claims(&object, &recallable).await.unwrap();
    for c in &filtered {
        assert_ne!(c.status, ClaimStatus::Forgotten);
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 10: Forgetting twice → Conflict.  Confirming forgotten → Conflict.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn forgetting_twice_conflict() {
    let root = temp_root("t10");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("bye").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    store
        .forget_claim(&id, ForgetReason::UserRequest)
        .await
        .unwrap();

    let err = store
        .forget_claim(&id, ForgetReason::Incorrect)
        .await
        .unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    let err = store.confirm_claim(&id).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 11: Re-proposing identical payload after forget → Duplicate { Forgotten }
// ---------------------------------------------------------------------------
#[tokio::test]
async fn repropose_after_forget_duplicates() {
    let root = temp_root("t11");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let payload = ClaimPayload::table_alias("resurrect").unwrap();
    let id = propose_confirmed(&store, &object, payload.clone()).await;
    store
        .forget_claim(&id, ForgetReason::Privacy)
        .await
        .unwrap();

    let request = ProposeClaim {
        object: object.clone(),
        fingerprint: fingerprint(),
        payload: payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: Vec::new(),
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Duplicate { id: did, status } => {
            assert_eq!(did, id);
            assert_eq!(status, ClaimStatus::Forgotten);
        }
        other => panic!("expected Duplicate, got {other:?}"),
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 12: Every operation on unknown ClaimId → NotFound
// ---------------------------------------------------------------------------
#[tokio::test]
async fn unknown_id_not_found() {
    let root = temp_root("t12");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let real = propose_confirmed(&store, &object, ClaimPayload::table_alias("real").unwrap()).await;

    let mut chars: Vec<char> = real.as_str().chars().collect();
    *chars.last_mut().unwrap() = match chars.last().unwrap() {
        '0'..='8' => std::char::from_u32(*chars.last().unwrap() as u32 + 1).unwrap(),
        '9' => 'a',
        'a'..='e' => std::char::from_u32(*chars.last().unwrap() as u32 + 1).unwrap(),
        'f' => '0',
        _ => '0',
    };
    let fake: ClaimId = ClaimId::parse(&chars.into_iter().collect::<String>()).unwrap();

    assert_eq!(
        store.confirm_claim(&fake).await.unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store
            .edit_claim(&fake, ClaimPayload::table_alias("x").unwrap())
            .await
            .unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store.reject_claim(&fake).await.unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store
            .forget_claim(&fake, ForgetReason::UserRequest)
            .await
            .unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store.mark_stale(&fake).await.unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(
        store.claim_events(&fake, 10).await.unwrap_err(),
        StoreError::NotFound
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 13: claim_events returns full ordered history
// ---------------------------------------------------------------------------
#[tokio::test]
async fn claim_events_full_history() {
    let root = temp_root("t13");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("history test").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    let _ = store.confirm_claim(&id).await.unwrap();
    let _ = store
        .edit_claim(&id, ClaimPayload::table_description("history v2").unwrap())
        .await
        .unwrap();
    store
        .forget_claim(&id, ForgetReason::Obsolete)
        .await
        .unwrap();

    let events = store.claim_events(&id, 100).await.unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].kind, ContractEventKind::Proposed);
    assert_eq!(events[0].from_status, None);
    assert_eq!(events[0].to_status, Some(ClaimStatus::Candidate));
    assert_eq!(events[1].kind, ContractEventKind::Confirmed);
    assert_eq!(events[1].from_status, Some(ClaimStatus::Candidate));
    assert_eq!(events[1].to_status, Some(ClaimStatus::Confirmed));
    assert_eq!(events[2].kind, ContractEventKind::Edited);
    assert_eq!(events[2].from_status, Some(ClaimStatus::Confirmed));
    assert_eq!(events[2].to_status, Some(ClaimStatus::Confirmed));
    assert_eq!(events[3].kind, ContractEventKind::Forgotten);
    assert_eq!(events[3].from_status, Some(ClaimStatus::Confirmed));
    assert_eq!(events[3].to_status, Some(ClaimStatus::Forgotten));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 14: Events carry no content
// ---------------------------------------------------------------------------
#[tokio::test]
async fn events_carry_no_content() {
    let root = temp_root("t14");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let sentinel = "SENTINELTEXT";
    let payload = ClaimPayload::table_description(format!("{sentinel} the events table")).unwrap();
    let id = propose_confirmed(&store, &object, payload).await;
    let _ = store
        .edit_claim(
            &id,
            ClaimPayload::table_description(format!("{sentinel} v2")).unwrap(),
        )
        .await
        .unwrap();

    let events = store.claim_events(&id, 10).await.unwrap();
    for event in &events {
        let debug = format!("{:?}", event);
        assert!(
            !debug.contains(sentinel),
            "event debug output contains sentinel: {debug}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 15: Atomicity — after Conflict, no new event row appended
// ---------------------------------------------------------------------------
#[tokio::test]
async fn atomicity_on_conflict() {
    let root = temp_root("t15");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");

    let id = propose_candidate(
        &store,
        &object,
        ClaimPayload::table_description("atomic").unwrap(),
        ClaimOrigin::UserExplicit,
    )
    .await;

    let pre = event_count(&db, id.as_str()).await;
    assert_eq!(pre, 1);

    let _ = store.confirm_claim(&id).await.unwrap();
    assert_eq!(event_count(&db, id.as_str()).await, 2);

    let _ = store.confirm_claim(&id).await.unwrap_err();
    assert_eq!(
        event_count(&db, id.as_str()).await,
        2,
        "Conflict did not roll back the event insert"
    );

    let _ = fs::remove_dir_all(root);
}

// --- Review addition: forget_claim accepted a ForgetReason and discarded it, so
// --- "why did this stop appearing?" — the question ADR 0002 keeps tombstones to
// --- answer — was unanswerable. The reason is a closed enum, so persisting it
// --- audits the deletion without storing anything the user typed.
#[tokio::test]
async fn forgetting_records_why_without_recording_what() {
    let root = temp_root("reason");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let object = object_ref(&profile_a(), "events");
    const SENTINEL: &str = "SENTINELTEXT";
    let payload =
        ClaimPayload::table_description(format!("{SENTINEL} rows are per order")).unwrap();
    let id = propose_confirmed(&store, &object, payload).await;

    store
        .forget_claim(&id, ForgetReason::Privacy)
        .await
        .unwrap();

    let events = store.claim_events(&id, 100).await.unwrap();
    let forgotten = events
        .iter()
        .find(|event| event.kind == ContractEventKind::Forgotten)
        .expect("a forgotten event was appended");
    assert_eq!(forgotten.reason, Some(ForgetReason::Privacy));
    assert_eq!(forgotten.to_status, Some(ClaimStatus::Forgotten));

    // Every other transition leaves it unset — a reason belongs only to a deletion.
    for event in events
        .iter()
        .filter(|e| e.kind != ContractEventKind::Forgotten)
    {
        assert_eq!(event.reason, None, "{:?} carried a reason", event.kind);
    }
    // The audit trail still says nothing about the claim's content.
    for event in &events {
        assert!(
            !format!("{event:?}").contains(SENTINEL),
            "event debug output leaked claim text",
        );
    }
    let _ = fs::remove_dir_all(root);
}
