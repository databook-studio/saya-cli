use super::{
    event_projection_storage::event,
    job_metadata_storage::{create_job, temp_root},
    token_charge_reservations::{reservation, settlement},
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetSettlementV1, CodingEventKind, JobState,
};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{fs, num::NonZeroU64, path::Path};

async fn pool(path: &Path) -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .unwrap()
}

async fn running(
    label: &str,
) -> (
    std::path::PathBuf,
    SqliteStateStore,
    saya_store::CodingJobRecord,
) {
    let root = temp_root(label);
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, label).await;
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
    (root, store, job)
}

async fn reserve_and_settle(store: &SqliteStateStore, job: &saya_types::JobId) {
    store
        .reserve_coding_budget(event(
            2,
            job,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(job, 2, None)),
            },
        ))
        .await
        .unwrap();
    store
        .settle_coding_budget(event(
            3,
            job,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(job, 2, 3)),
            },
        ))
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_fail_closed_for_tampered_settlement_keys_future_payloads_and_missing_refs() {
    let (root, store, job) = running("r13a-corrupt").await;
    let db = root.join("state.sqlite3");
    reserve_and_settle(&store, &job.job_id).await;
    store.close().await;
    let raw = pool(&db).await;
    let wrong = BudgetSettlementV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::new(3).unwrap(),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::LogicalRequests,
                None,
                None,
                BudgetObservationValue::Measured(1),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let wrong_event = event(
        3,
        &job.job_id,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(wrong),
        },
    );
    sqlx::query("UPDATE coding_events SET event_json=? WHERE job_id=? AND sequence=3")
        .bind(String::from_utf8(wrong_event.to_json_bytes().unwrap()).unwrap())
        .bind(job.job_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let reopened = SqliteStateStore::new(&db);
    assert_eq!(
        reopened
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        reopened
            .list_coding_budget_reservations(&job.job_id, None, 1)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    reopened.close().await;

    let raw = pool(&db).await;
    sqlx::query("UPDATE coding_events SET event_json=replace(event_json, '\"version\":1', '\"version\":2') WHERE job_id=? AND sequence=3")
        .bind(job.job_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let future = SqliteStateStore::new(&db);
    assert_eq!(
        future
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap_err(),
        StoreError::VersionUnsupported
    );
    future.close().await;

    let raw = pool(&db).await;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("DELETE FROM coding_events WHERE job_id=? AND sequence=3")
        .bind(job.job_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let missing = SqliteStateStore::new(&db);
    assert_eq!(
        missing
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    missing.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn projection_trigger_rolls_back_event_then_allows_retry() {
    let (root, store, job) = running("r13a-trigger").await;
    let db = root.join("state.sqlite3");
    store.close().await;
    let raw = pool(&db).await;
    sqlx::query("CREATE TRIGGER reject_budget_projection BEFORE INSERT ON coding_budget_reservations BEGIN SELECT RAISE(ABORT, 'reject'); END")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let rejected = SqliteStateStore::new(&db);
    let candidate = event(
        2,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation(&job.job_id, 2, None)),
        },
    );
    assert_eq!(
        rejected
            .reserve_coding_budget(candidate.clone())
            .await
            .unwrap_err(),
        StoreError::Unavailable
    );
    rejected.close().await;
    let raw = pool(&db).await;
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM coding_events WHERE job_id=?")
        .bind(job.job_id.as_str())
        .fetch_one(&raw)
        .await
        .unwrap();
    let rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM coding_budget_reservations WHERE job_id=?")
            .bind(job.job_id.as_str())
            .fetch_one(&raw)
            .await
            .unwrap();
    assert_eq!((events, rows), (1, 0));
    sqlx::query("DROP TRIGGER reject_budget_projection")
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;
    let retried = SqliteStateStore::new(&db);
    retried.reserve_coding_budget(candidate).await.unwrap();
    retried.close().await;
    let _ = fs::remove_dir_all(root);
}
