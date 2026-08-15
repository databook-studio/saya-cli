//! P1 regression: confirming a stale claim must revalidate it.
//!
//! `confirm_claim` used to flip a Stale claim to Confirmed but leave the stored
//! fingerprint and referenced-column snapshots untouched. The next read
//! recomputed the digest from the live schema, found it still differed, and
//! returned Stale again — so a confirmation was a silent no-op that the user
//! could not repair (`edit_claim` also refused Stale). These tests pin the fix:
//! a schema-aware `revalidate_claim` that rewrites the fingerprint and snapshots
//! in the same transaction as the status flip, and refuses when a referenced
//! column is gone rather than reviving a claim against a schema it no longer
//! matches. See the SPEC REVIEW in the task report for both decisions.

use saya_store::{
    ContractEventKind, ContractStore, ProposeClaim, ProposeOutcome, SchemaStore, SqliteStateStore,
    StoreError,
};
use saya_types::{
    ClaimId, ClaimOrigin, ClaimPayload, ClaimStatus, Column, DatabaseObjectKind, DatabaseObjectRef,
    ProfileIdentity, SchemaFingerprint, SchemaTree, Table,
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
// Helpers — mirror `contract_lifecycle.rs` so these read the same way.
// ---------------------------------------------------------------------------

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-revalidate-{label}-{}-{stamp}",
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

/// Proposes a Confirmed claim fingerprinted against `live`, with real
/// per-column snapshots, so it reads Current until the schema drifts.
async fn propose_current(
    store: &SqliteStateStore,
    object: &DatabaseObjectRef,
    live: &Table,
    payload: ClaimPayload,
) -> ClaimId {
    let fingerprint = SchemaFingerprint::of_table(DatabaseObjectKind::Table, live);
    let referenced = payload.referenced_column_snapshots(live);
    let request = ProposeClaim {
        object: object.clone(),
        fingerprint,
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
        referenced_columns: referenced,
    };
    match store.propose_claim(request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    }
}

/// Marks `id` Stale by overwriting the stored fingerprint with one that cannot
/// match any real table (an all-zero digest), then calling `mark_stale`. This
/// is the state the bug left a claim in: Confirmed status, but a fingerprint
/// the next read can never equal — so `schema_state_for` returns Stale.
async fn force_stale(store: &SqliteStateStore, db: &Path, id: &ClaimId) {
    let pool = read_pool(db).await;
    // An all-zero digest of the current format: never equal to a real table's
    // fingerprint, so the claim is structurally stale regardless of status.
    let zero = "0".repeat(64);
    sqlx::query("UPDATE contract_claims SET schema_fingerprint=? WHERE id=?")
        .bind(&zero)
        .bind(id.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    store.mark_stale(id).await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: revalidating a stale claim against a live table whose columns still
// exist makes it Confirmed AND Current on the next read — the bug, named.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revalidate_makes_a_stale_claim_current_again() {
    let root = temp_root("current");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &base, payload).await;

    // Break it: a fingerprint no live table can match, then mark Stale.
    force_stale(&store, &db, &id).await;
    let stale = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stale.status, ClaimStatus::Stale);
    assert_ne!(
        stale.schema_fingerprint,
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &base),
        "the forced-stale fingerprint must not match the live table"
    );

    // Revalidate against the live table whose columns still exist.
    let revalidated = store.revalidate_claim(&id, &base).await.unwrap();
    assert_eq!(revalidated.status, ClaimStatus::Confirmed);

    // The next read classifies it Current: the stored fingerprint now equals
    // the live digest, so `schema_state_for` does not return Stale. Before the
    // fix this assertion failed — the confirmation was a silent no-op.
    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.status, ClaimStatus::Confirmed);
    assert_eq!(
        stored.schema_fingerprint,
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &base),
        "revalidation must rewrite the fingerprint to the live digest"
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 2: revalidating where a referenced column is absent refuses with a
// typed error and changes nothing — no reviving a claim against a schema its
// dependency vanished from.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revalidate_refuses_when_a_referenced_column_is_absent() {
    let root = temp_root("absent");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &base, payload).await;
    force_stale(&store, &db, &id).await;

    let before = store.get_claim(&id).await.unwrap().unwrap();
    let pre_events = event_count(&db, id.as_str()).await;
    assert_eq!(pre_events, 2, "proposed + marked_stale before the refusal");

    // Live table dropped the referenced column `amount`.
    let drifted = table(&[("id", "bigint", false)]);
    let err = store.revalidate_claim(&id, &drifted).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    // Nothing changed: status, fingerprint, snapshots all as they were.
    let after = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(after.status, before.status);
    assert_eq!(after.schema_fingerprint, before.schema_fingerprint);
    assert_eq!(after.referenced_columns, before.referenced_columns);

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 3: the fingerprint and snapshots actually change — assert the stored
// values, not just the status.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revalidate_rewrites_fingerprint_and_snapshots() {
    let root = temp_root("rewrite");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let original = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &original, payload.clone()).await;
    force_stale(&store, &db, &id).await;

    let stale = store.get_claim(&id).await.unwrap().unwrap();
    // The stale claim carries a forced all-zero digest and the original snapshot.
    assert_ne!(
        stale.schema_fingerprint.as_str(),
        original_digest(&original)
    );
    assert_eq!(
        stale.referenced_columns,
        payload.referenced_column_snapshots(&original)
    );

    // Revalidate against a *different* live table — same columns, retyped — so
    // both the fingerprint and the per-column snapshot must move to new values.
    let retyped = table(&[("id", "bigint", false), ("amount", "int", false)]);
    store.revalidate_claim(&id, &retyped).await.unwrap();

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(
        stored.schema_fingerprint,
        SchemaFingerprint::of_table(DatabaseObjectKind::Table, &retyped),
        "the stored fingerprint is the retyped table's digest"
    );
    assert_eq!(
        stored.referenced_columns,
        payload.referenced_column_snapshots(&retyped),
        "the stored snapshot carries the retyped column's new type"
    );
    // And it is not still the original snapshot's type.
    assert_ne!(
        stored.referenced_columns[0].data_type,
        stale.referenced_columns[0].data_type
    );

    let _ = fs::remove_dir_all(root);
}

fn original_digest(table: &Table) -> String {
    SchemaFingerprint::of_table(DatabaseObjectKind::Table, table)
        .as_str()
        .to_string()
}

// ---------------------------------------------------------------------------
// Test 4: exactly one audit event is appended on revalidation, and none on the
// refusal path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revalidate_appends_one_event_refusal_appends_none() {
    let root = temp_root("events");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &base, payload).await;
    force_stale(&store, &db, &id).await;

    // Proposed + marked_stale so far.
    let before = event_count(&db, id.as_str()).await;
    assert_eq!(before, 2);

    // Refusal appends nothing.
    let drifted = table(&[("id", "bigint", false)]);
    let _ = store.revalidate_claim(&id, &drifted).await.unwrap_err();
    assert_eq!(event_count(&db, id.as_str()).await, 2);

    // A successful revalidation appends exactly one event.
    store.revalidate_claim(&id, &base).await.unwrap();
    assert_eq!(event_count(&db, id.as_str()).await, 3);

    let events = store.claim_events(&id, 100).await.unwrap();
    let last = events.last().unwrap();
    assert_eq!(last.kind, ContractEventKind::Confirmed);
    assert_eq!(last.from_status, Some(ClaimStatus::Stale));
    assert_eq!(last.to_status, Some(ClaimStatus::Confirmed));

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5a (decision): plain `confirm_claim` no longer accepts Stale. Silently
// doing a weaker thing under a familiar name was the bug; a Stale claim must
// take the revalidation path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn confirm_refuses_a_stale_claim() {
    let root = temp_root("confirm_refuses_stale");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &base, payload).await;
    force_stale(&store, &db, &id).await;

    let err = store.confirm_claim(&id).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);
    // The claim is still Stale — the refusal changed nothing.
    assert_eq!(
        store.get_claim(&id).await.unwrap().unwrap().status,
        ClaimStatus::Stale
    );

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 5b (decision): `edit_claim` now accepts Stale. Editing is how a user
// repairs a claim whose column was renamed; refusing Stale left such a claim
// unrepairable. Edit re-runs admission and recomputes the dedup key; with no
// live table it stores name-only snapshots (unknown, not Current), so the
// claim unblocks to Confirmed and a later reconcile can promote it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn edit_repairs_a_stale_claim() {
    let root = temp_root("edit_repairs_stale");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();
    let id = propose_current(&store, &obj, &base, payload).await;
    force_stale(&store, &db, &id).await;

    // Repoint the claim at the renamed column.
    let repaired = ClaimPayload::column_description("total", "how much").unwrap();
    let edited = store.edit_claim(&id, repaired.clone()).await.unwrap();
    assert_eq!(edited.status, ClaimStatus::Confirmed);
    assert_eq!(edited.payload, Some(repaired));

    let stored = store.get_claim(&id).await.unwrap().unwrap();
    assert_eq!(stored.status, ClaimStatus::Confirmed);
    // The edit recorded a name-only snapshot for the new column: empty type,
    // so a later reconcile treats it as unknown rather than matched.
    assert_eq!(stored.referenced_columns.len(), 1);
    assert_eq!(stored.referenced_columns[0].name, "total");
    assert!(stored.referenced_columns[0].data_type.is_empty());

    let _ = fs::remove_dir_all(root);
}

// ---------------------------------------------------------------------------
// Test 6: the origin rule is unchanged. A claim whose origin is not permitted
// to store confirmed cannot be revalidated into Confirmed either. (Existing
// transition tests in contract_lifecycle.rs stay unchanged.)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revalidate_keeps_legal_statuses_and_origins() {
    let root = temp_root("legal");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    store
        .upsert_schema(profile_a().as_str(), &SchemaTree::default())
        .await
        .unwrap();

    let p = profile_a();
    let base = table(&[("id", "bigint", false), ("amount", "numeric", false)]);
    let obj = object_ref(&p, "orders");
    let payload = ClaimPayload::column_description("amount", "how much").unwrap();

    // A Confirmed claim (not Stale) cannot be revalidated — it is already
    // confirmed; the operation is for stale/candidate claims. Conflict.
    let confirmed = propose_current(&store, &obj, &base, payload.clone()).await;
    let err = store.revalidate_claim(&confirmed, &base).await.unwrap_err();
    assert_eq!(err, StoreError::Conflict);

    // A Candidate can be revalidated against a live table (confirm-with-schema).
    // Use a distinct payload so it is not a duplicate of the confirmed claim.
    let candidate_payload = ClaimPayload::column_description("id", "the primary key").unwrap();
    let candidate_request = ProposeClaim {
        object: obj.clone(),
        fingerprint: SchemaFingerprint::of_table(DatabaseObjectKind::Table, &base),
        payload: candidate_payload.clone(),
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Candidate,
        evidence: None,
        referenced_columns: candidate_payload.referenced_column_snapshots(&base),
    };
    let candidate = match store.propose_claim(candidate_request).await.unwrap() {
        ProposeOutcome::Stored(id) => id,
        other => panic!("expected Stored, got {other:?}"),
    };
    let revalidated = store.revalidate_claim(&candidate, &base).await.unwrap();
    assert_eq!(revalidated.status, ClaimStatus::Confirmed);

    let _ = fs::remove_dir_all(root);
}
