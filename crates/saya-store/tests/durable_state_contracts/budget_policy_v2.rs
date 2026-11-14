use super::{
    job_metadata_storage::temp_root,
    policy_records::{policy, running_job},
};
use saya_store::{CodingBudgetPolicyAppend, SqliteStateStore, StoreError};
use saya_types::{BudgetLimitPolicyV2, CodingBudgetPolicy};
use std::fs;

#[tokio::test]
async fn uncapped_v2_policy_records_explicit_absence_and_reopens_stably() {
    let root = temp_root("r03b-uncapped-v2");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03b-uncapped-v2").await;
    let policy = BudgetLimitPolicyV2::uncapped(4, 4).unwrap();
    assert_eq!(
        store
            .record_coding_uncapped_budget_policy(&job, &policy)
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
    assert_eq!(
        stored.policy,
        CodingBudgetPolicy::UncappedV2(policy.clone())
    );
    assert_eq!(stored.policy_sha256, policy.digest());
    assert_eq!(
        reopened
            .record_coding_uncapped_budget_policy(&job, &policy)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::AlreadyPresentSame
    );
    let bytes = stored.policy.canonical_json_bytes().unwrap();
    let json = std::str::from_utf8(&bytes).unwrap();
    assert!(json.contains("\"logical_requests\":null"));
    assert!(json.contains("\"elapsed_millis\":null"));
    assert!(json.contains("\"model_slots\":4"));
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn v1_and_v2_policies_conflict_without_replacing_the_original_row() {
    let root = temp_root("r03b-policy-version-conflict");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r03b-policy-version-conflict").await;
    let bounded = policy(120);
    assert_eq!(
        store
            .record_coding_budget_policy(&job, &bounded)
            .await
            .unwrap(),
        CodingBudgetPolicyAppend::Recorded
    );
    assert_eq!(
        store
            .record_coding_uncapped_budget_policy(
                &job,
                &BudgetLimitPolicyV2::uncapped(4, 4).unwrap(),
            )
            .await
            .unwrap_err(),
        StoreError::PolicyConflict
    );
    let stored = store.get_coding_budget_policy(&job).await.unwrap().unwrap();
    assert_eq!(stored.policy, bounded);
    assert_eq!(stored.policy_sha256, bounded.digest());
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
