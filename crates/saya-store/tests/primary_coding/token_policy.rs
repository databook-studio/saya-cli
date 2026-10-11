use super::{
    r01::{create_job, temp_root},
    r02::event,
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    CodingEventKind, JobState, Sha256Digest, TokenBudgetPolicyData, TokenBudgetPolicyV1,
};
#[tokio::test]
async fn verified_token_policy_is_immutable_and_survives_reopen() {
    let root = temp_root("token-policy");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "token-policy").await;
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
    let data = TokenBudgetPolicyData {
        version: 1,
        total_tokens: 100,
        provider: "openai".into(),
        model: "verified-fixture".into(),
        endpoint_sha256: TokenBudgetPolicyV1::endpoint_digest(
            "https://provider.invalid/v1/chat/completions",
        )
        .unwrap(),
        verification_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        input_context_upper_bound: 10,
        output_tokens: 5,
    };
    let policy = TokenBudgetPolicyV1::new(data.clone()).unwrap();
    store
        .record_coding_token_policy(&job.job_id, &policy)
        .await
        .unwrap();
    store
        .record_coding_token_policy(&job.job_id, &policy)
        .await
        .unwrap();
    let other = TokenBudgetPolicyV1::new(TokenBudgetPolicyData {
        total_tokens: 200,
        ..data
    })
    .unwrap();
    assert_eq!(
        store
            .record_coding_token_policy(&job.job_id, &other)
            .await
            .unwrap_err(),
        StoreError::PolicyConflict
    );
    store.close().await;
    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened.get_coding_token_policy(&job.job_id).await.unwrap(),
        Some(policy)
    );
    reopened.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
