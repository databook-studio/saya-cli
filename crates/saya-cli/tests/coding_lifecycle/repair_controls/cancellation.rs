use super::super::{provider::Provider, support::Fixture};
use super::support::{completed, resume};
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingError};

#[tokio::test]
async fn corrupt_lineage_does_not_block_owned_durable_cancellation() {
    let provider = Provider::repair_controls();
    let (fixture, job, _) = completed(
        "repair-cancel-corrupt",
        &provider,
        vec!["tracked.txt".into()],
    )
    .await;
    // A run that ended with a final answer is `completed`, which is terminal
    // for control requests. Continue it, so the cancel below is requested for
    // a job that is actually running.
    resume(&fixture, &job, CancellationToken::new()).await;
    assert!(fixture.application.status(job.job_id()).await.is_ok());
    let options =
        sqlx::sqlite::SqliteConnectOptions::new().filename(fixture.root.join("state.sqlite3"));
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    let changed=sqlx::query("UPDATE coding_events SET event_json=json_set(event_json,'$.sequence',999) WHERE job_id=? AND json_extract(event_json,'$.event.type')='child_invocation' AND json_extract(event_json,'$.event.record.operation')='scope_bound'")
        .bind(job.job_id().as_str()).execute(&pool).await.unwrap();
    assert_eq!(changed.rows_affected(), 1);
    pool.close().await;
    assert!(
        fixture.application.status(job.job_id()).await.is_err(),
        "strict indexed authoritative lineage validation remains enabled"
    );
    let cancel = fixture.application.cancel(job.job_id()).await;
    assert!(
        cancel.is_ok(),
        "owned cancellation must not depend on corrupt derived status: {cancel:?}"
    );
    assert!(
        fixture
            .store
            .coding_cancel_requested(job.job_id())
            .await
            .unwrap()
    );
    assert!(
        fixture.application.status(job.job_id()).await.is_err(),
        "cancellation does not repair or conceal corrupt history"
    );
}
#[tokio::test]
async fn foreign_repository_cannot_request_cancellation_in_shared_store() {
    let provider = Provider::repair_controls();
    let (fixture, job, _) = completed(
        "repair-cancel-foreign",
        &provider,
        vec!["tracked.txt".into()],
    )
    .await;
    // The foreign application only proves the refusal; it runs no job, so it
    // takes the resolved coordinator mode too.
    let foreign = Fixture::new("repair-cancel-other-repository").await;
    let application = CodingApplication::open(
        foreign.runtime.clone(),
        fixture.store.clone(),
        &foreign.repository,
        &foreign.root.join("coding"),
        &foreign.git,
        &foreign.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(
        fixture
            .store
            .get_coding_job(job.job_id())
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        application.cancel(job.job_id()).await,
        Err(CodingError::NotOwned)
    ));
    assert!(
        !fixture
            .store
            .coding_cancel_requested(job.job_id())
            .await
            .unwrap()
    );
}
