use super::checkout_plan::{PLAN_BYTES, done, record, seeded};
use super::r03b::read_pool;
use saya_store::{SqliteStateStore, StoreError};
use sqlx::query;

/// A stored row whose `plan_bytes` no longer match the recorded digest is
/// refused by the read path with a typed `Invalid`, never returned as data.
#[tokio::test]
async fn corrupt_stored_row_reads_as_invalid() {
    let mut s = seeded("w05c1-corrupt", true).await;
    record(&s, PLAN_BYTES).await.unwrap();
    s.store.close().await;
    let raw = read_pool(&s.db).await;
    query("UPDATE coding_checkout_plans SET plan_bytes = ? WHERE job_id = ? AND managed_checkout_id = ?")
        .bind(b"corrupted-plan-bytes".to_vec())
        .bind(s.job.as_str())
        .bind(s.checkout.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    s.store = SqliteStateStore::new(&s.db);
    assert_eq!(
        s.store
            .get_checkout_plan_bytes(&s.job, &s.checkout)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    done(s).await;
}

/// The foreign key on `(job_id, managed_checkout_id)` refuses a plan row whose
/// checkout does not exist: the database rejects the insert outright.
#[tokio::test]
async fn orphan_plan_row_is_rejected_by_the_foreign_key() {
    let s = seeded("w05c1-orphan", false).await;
    let raw = read_pool(&s.db).await;
    let error = query(
        "INSERT INTO coding_checkout_plans(job_id, managed_checkout_id, plan_sha256, plan_bytes) VALUES (?, ?, ?, ?)",
    )
    .bind(s.job.as_str())
    .bind(s.checkout.as_str())
    .bind("digest-placeholder")
    .bind(PLAN_BYTES.to_vec())
    .execute(&raw)
    .await
    .unwrap_err();
    raw.close().await;
    assert!(
        error.to_string().contains("FOREIGN KEY constraint failed"),
        "insert must be refused by the foreign key, got: {error}"
    );
    done(s).await;
}
