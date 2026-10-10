use super::{
    r01::{create_job, temp_root},
    r02::event,
};
use saya_store::{CodingBudgetPolicyAppend, SqliteStateStore, StoreError};
use saya_types::{BudgetLimitPolicyV1, CodingEventKind, JobId, JobState};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::fs;

pub(super) fn policy(logical_requests: u64) -> BudgetLimitPolicyV1 {
    BudgetLimitPolicyV1::new(logical_requests, 240, 80, 4, 2, 1_800_000).unwrap()
}

pub(super) async fn running_job(store: &SqliteStateStore, tag: &str) -> JobId {
    let job = create_job(store, tag).await;
    store
        .append_coding_event(event(
            1,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    job.job_id
}

pub(super) async fn read_pool(db: &std::path::Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(db))
        .await
        .unwrap()
}

#[tokio::test]
async fn policy_recorded_once_and_reopen_stable() {
    let root = temp_root("r03b-reopen");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03b-reopen").await;
    let policy = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    let stored = reopened
        .get_coding_budget_policy(&job)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.policy, policy);
    assert_eq!(stored.policy_sha256, policy.digest());
    assert_eq!(
        reopened
            .get_coding_budget_policy(&saya_types::JobId::parse("job-r03b-missing").unwrap())
            .await
            .unwrap(),
        None
    );
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn policy_same_digest_is_idempotent() {
    let root = temp_root("r03b-idempotent");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r03b-idempotent").await;
    let policy = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::AlreadyPresentSame
    );
    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, policy);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn policy_conflicting_digest_fails_closed() {
    let root = temp_root("r03b-conflict");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r03b-conflict").await;
    let original = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &original)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &policy(121))
            .await
            .unwrap_err(),
        StoreError::PolicyConflict
    );
    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, original);
    assert_eq!(stored.policy_sha256, original.digest());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn policy_requires_running_job() {
    let root = temp_root("r03b-state");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let queued = create_job(&store, "r03b-state").await;
    let policy = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&queued.job_id, &policy)
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .record_coding_budget_policy(
                &saya_types::JobId::parse("job-r03b-absent").unwrap(),
                &policy
            )
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .get_coding_budget_policy(&queued.job_id)
            .await
            .unwrap(),
        None
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn tampered_policy_row_is_a_typed_corruption_error() {
    let root = temp_root("r03b-tamper");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let json_tampered = running_job(&store, "r03b-tamper-json").await;
    let digest_tampered = running_job(&store, "r03b-tamper-sha").await;
    let malformed = running_job(&store, "r03b-tamper-malformed").await;
    let policy = policy(120);
    for job in [&json_tampered, &digest_tampered, &malformed] {
        assert_eq!(
            store
                .record_coding_budget_policy(job, &policy)
                .await
                .unwrap(),
            CodingBudgetPolicyAppend::Recorded
        );
    }
    store.close().await;

    let different_json = r#"{"version":1,"logical_requests":121,"transport_attempts":240,"tool_calls":80,"model_slots":4,"child_slots":2,"deadline_millis":1800000}"#;
    let zero_digest = "0".repeat(64);
    let pool = read_pool(&db).await;
    sqlx::query("UPDATE coding_budget_policies SET policy_json=? WHERE job_id=?")
        .bind(different_json)
        .bind(json_tampered.as_str())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE coding_budget_policies SET policy_sha256=? WHERE job_id=?")
        .bind(zero_digest)
        .bind(digest_tampered.as_str())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE coding_budget_policies SET policy_json='not json' WHERE job_id=?")
        .bind(malformed.as_str())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let reopened = SqliteStateStore::new(&db);
    for job in [&json_tampered, &digest_tampered, &malformed] {
        assert_eq!(
            reopened.get_coding_budget_policy(job).await.unwrap_err(),
            StoreError::Invalid
        );
    }
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
