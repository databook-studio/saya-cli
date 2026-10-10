//! Retry and concurrency proofs for the coding budget policy record: a
//! corrupted existing row must never yield a resume proof, and concurrent
//! recorders of one job must converge on exactly one surviving row.

use super::r01::temp_root;
use super::r03b::{policy, read_pool, running_job};
use saya_store::{CodingBudgetPolicyAppend, SqliteStateStore, StoreError};
use saya_types::BudgetLimitPolicyV1;
use sqlx::SqlitePool;
use std::fs;

/// The record retry path must revalidate the stored row with the same checks
/// the read path applies. A row whose `policy_json` was corrupted while its
/// `policy_sha256` column stayed unchanged is typed corruption
/// (`StoreError::Invalid`) — never a successful `AlreadyPresentSame` proof —
/// for both the record retry and the read.
#[tokio::test]
async fn retry_on_corrupted_policy_json_is_a_typed_corruption_error() {
    let root = temp_root("r03bx-corrupt-retry");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03bx-corrupt-retry").await;
    let policy = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    store.close().await;

    let pool = read_pool(&db).await;
    sqlx::query("UPDATE coding_budget_policies SET policy_json=? WHERE job_id=?")
        .bind(
            r#"{"version":1,"logical_requests":121,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000}"#,
        )
        .bind(job.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened
            .record_coding_budget_policy(&job, &policy)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        reopened.get_coding_budget_policy(&job).await.unwrap_err(),
        StoreError::Invalid
    );
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Two recorders racing on the same job with the same policy serialize on the
/// store's `BEGIN IMMEDIATE` write: both succeed — one `Recorded`, one
/// `AlreadyPresentSame` — and exactly one row survives.
#[tokio::test]
async fn concurrent_recorders_converge_on_one_row() {
    let root = temp_root("r03bx-race-same");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03bx-race-same").await;
    let policy = policy(120);

    let mut handles = Vec::new();
    for _ in 0..2 {
        let store = store.clone();
        let job = job.clone();
        let policy = policy.clone();
        handles.push(tokio::spawn(async move {
            store.record_coding_budget_policy(&job, &policy).await
        }));
    }
    let (mut recorded, mut same) = (0, 0);
    for handle in handles {
        match handle.await.unwrap().unwrap() {
            CodingBudgetPolicyAppend::Recorded => recorded += 1,
            CodingBudgetPolicyAppend::AlreadyPresentSame => same += 1,
        }
    }
    assert_eq!((recorded, same), (1, 1), "one insert wins, one resumes");

    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, policy);
    assert_eq!(stored.policy_sha256, policy.digest());
    assert_eq!(policy_row_count(&db).await, 1);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// Two recorders racing with different policies: exactly one inserts, the
/// other is refused with a typed `PolicyConflict`, and the surviving row is
/// the winner's.
#[tokio::test]
async fn concurrent_recorders_with_different_policies_leave_one_row() {
    let root = temp_root("r03bx-race-diff");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03bx-race-diff").await;

    let mut handles = Vec::new();
    for logical_requests in [120_u64, 121] {
        let store = store.clone();
        let job = job.clone();
        let policy = policy(logical_requests);
        handles.push(tokio::spawn(async move {
            let outcome = store.record_coding_budget_policy(&job, &policy).await;
            (policy, outcome)
        }));
    }
    let mut winner: Option<BudgetLimitPolicyV1> = None;
    for handle in handles {
        let (policy, outcome) = handle.await.unwrap();
        match outcome {
            Ok(CodingBudgetPolicyAppend::Recorded) => {
                assert!(winner.is_none(), "two recorders cannot both insert");
                winner = Some(policy);
            }
            Ok(other) => panic!("the refused recorder saw {other:?}"),
            Err(StoreError::PolicyConflict) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    let winner = winner.expect("exactly one recorder wins");

    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, winner);
    assert_eq!(stored.policy_sha256, winner.digest());
    assert_eq!(policy_row_count(&db).await, 1);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

/// After an `AlreadyPresentSame` retry, the observable claim is that no
/// write transaction is left holding the database: a different job's
/// record succeeds immediately without a busy/lock error, and the first
/// job still has exactly one row with the original digest. Whether the
/// retry itself committed or rolled back its empty transaction is not
/// observable here — both produce these outcomes — so this test proves
/// only the absence of a leaked write transaction.
#[tokio::test]
async fn already_present_same_retry_does_not_block_the_next_writer_and_is_stable() {
    let root = temp_root("r03bx-retry-tx");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03bx-retry-tx-a").await;
    let first_policy = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &first_policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    for _ in 0..3 {
        assert_eq!(
            store
                .record_coding_budget_policy(&job, &first_policy)
                .await
                .unwrap(),
            CodingBudgetPolicyAppend::AlreadyPresentSame
        );
    }

    let other_job = running_job(&store, "r03bx-retry-tx-b").await;
    let other_policy = policy(121);
    assert_eq!(
        store
            .record_coding_budget_policy(&other_job, &other_policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );

    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, first_policy);
    assert_eq!(stored.policy_sha256, first_policy.digest());
    let job_rows: i64 = {
        let pool = read_pool(&db).await;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_policies WHERE job_id=?")
                .bind(job.as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
        pool.close().await;
        count
    };
    assert_eq!(job_rows, 1, "first job keeps one row");
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

async fn policy_row_count(db: &std::path::Path) -> i64 {
    let pool: SqlitePool = read_pool(db).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_policies")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    count
}
