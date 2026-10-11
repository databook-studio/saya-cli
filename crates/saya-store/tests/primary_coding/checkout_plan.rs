use super::r01::temp_root;
use super::r03b::{read_pool, running_job};
use saya_store::{
    CheckoutPlanAppend, MAX_CHECKOUT_PLAN_BYTES, SqliteStateStore, StoreError, StoredCheckoutPlan,
};
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use sha2::{Digest, Sha256};
use sqlx::{query, query_scalar};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const PLAN_BYTES: &[u8] = b"plan-manifest-v1";
const CONFLICT_BYTES: &[u8] = b"plan-manifest-v2";

pub(super) struct Seeded {
    pub(super) root: PathBuf,
    pub(super) db: PathBuf,
    pub(super) store: SqliteStateStore,
    pub(super) job: JobId,
    pub(super) checkout: ManagedCheckoutId,
}

fn digest_of(bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&hex).unwrap()
}

fn plan(bytes: &[u8]) -> StoredCheckoutPlan {
    StoredCheckoutPlan {
        plan_sha256: digest_of(bytes),
        plan_bytes: bytes.to_vec(),
    }
}

/// Running job, with an optional managed-checkout row whose FK target is the
/// running event at sequence 1.
pub(super) async fn seeded(tag: &str, with_checkout: bool) -> Seeded {
    let root = temp_root(tag);
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, tag).await;
    let checkout = ManagedCheckoutId::parse(&format!("checkout-{tag}")).unwrap();
    if with_checkout {
        let pool = read_pool(&db).await;
        query(
            "INSERT INTO coding_managed_checkouts(job_id, checkout_id, created_by_sequence, latest_operation_sequence) VALUES (?, ?, 1, 1)",
        )
        .bind(job.as_str())
        .bind(checkout.as_str())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
    Seeded {
        root,
        db,
        store,
        job,
        checkout,
    }
}

pub(super) async fn record(s: &Seeded, bytes: &[u8]) -> Result<CheckoutPlanAppend, StoreError> {
    s.store
        .record_checkout_plan_bytes(&s.job, &s.checkout, &digest_of(bytes), bytes)
        .await
}

async fn stored(s: &Seeded) -> Option<StoredCheckoutPlan> {
    s.store
        .get_checkout_plan_bytes(&s.job, &s.checkout)
        .await
        .unwrap()
}

async fn user_version(db: &Path) -> i64 {
    let pool = read_pool(db).await;
    let version: i64 = query_scalar("PRAGMA user_version")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    version
}

pub(super) async fn done(s: Seeded) {
    s.store.close().await;
    let _ = fs::remove_dir_all(s.root);
}

#[tokio::test]
async fn records_once_and_reads_back() {
    let s = seeded("w05c1-read", true).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    assert_eq!(stored(&s).await, Some(plan(PLAN_BYTES)));
    done(s).await;
}

#[tokio::test]
async fn same_digest_is_idempotent() {
    let s = seeded("w05c1-idem", true).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::AlreadyPresentSame
    );
    assert_eq!(stored(&s).await, Some(plan(PLAN_BYTES)));
    done(s).await;
}

#[tokio::test]
async fn conflicting_digest_refuses_and_keeps_original() {
    let s = seeded("w05c1-conflict", true).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    assert_eq!(
        record(&s, CONFLICT_BYTES).await.unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(stored(&s).await, Some(plan(PLAN_BYTES)));
    done(s).await;
}

#[tokio::test]
async fn digest_must_match_bytes() {
    let s = seeded("w05c1-digest", true).await;
    assert_eq!(
        s.store
            .record_checkout_plan_bytes(&s.job, &s.checkout, &digest_of(b"other-plan"), PLAN_BYTES)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(stored(&s).await, None);
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    done(s).await;
}

#[tokio::test]
async fn oversize_refused() {
    let s = seeded("w05c1-oversize", true).await;
    let max_bytes = vec![0u8; MAX_CHECKOUT_PLAN_BYTES];
    assert_eq!(
        record(&s, &max_bytes).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    let oversize = vec![0u8; MAX_CHECKOUT_PLAN_BYTES + 1];
    assert_eq!(
        record(&s, &oversize).await.unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(stored(&s).await, Some(plan(&max_bytes)));
    done(s).await;
}

#[tokio::test]
async fn checkout_must_exist() {
    let s = seeded("w05c1-absent", false).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap_err(),
        StoreError::NotFound
    );
    let missing = JobId::parse("job-w05c1-never").unwrap();
    assert_eq!(
        s.store
            .record_checkout_plan_bytes(&missing, &s.checkout, &digest_of(PLAN_BYTES), PLAN_BYTES)
            .await
            .unwrap_err(),
        StoreError::NotFound
    );
    assert_eq!(stored(&s).await, None);
    done(s).await;
}

#[tokio::test]
async fn reopen_is_stable() {
    let mut s = seeded("w05c1-reopen", true).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    s.store.close().await;
    s.store = SqliteStateStore::new(&s.db);
    assert_eq!(stored(&s).await, Some(plan(PLAN_BYTES)));
    let absent = ManagedCheckoutId::parse("checkout-w05c1-never").unwrap();
    assert_eq!(
        s.store
            .get_checkout_plan_bytes(&s.job, &absent)
            .await
            .unwrap(),
        None
    );
    done(s).await;
}

#[tokio::test]
async fn checkout_plan_survives_latest_schema_migration() {
    let mut s = seeded("w05c1-migrate", true).await;
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    s.store.close().await;
    assert_eq!(user_version(&s.db).await, 18);
    let raw = read_pool(&s.db).await;
    for table in [
        "coding_token_policies",
        "coding_transport_observations",
        "coding_cancel_requests",
        "coding_runtime_authority",
        "coding_completions",
    ] {
        query(&format!("DROP TABLE {table}"))
            .execute(&raw)
            .await
            .unwrap();
    }
    query("DROP TABLE coding_checkout_plans")
        .execute(&raw)
        .await
        .unwrap();
    query("PRAGMA user_version = 15")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    assert_eq!(user_version(&s.db).await, 15);
    s.store = SqliteStateStore::new(&s.db);
    assert_eq!(stored(&s).await, None);
    assert_eq!(user_version(&s.db).await, 18);
    assert_eq!(
        record(&s, PLAN_BYTES).await.unwrap(),
        CheckoutPlanAppend::Recorded
    );
    done(s).await;
}
