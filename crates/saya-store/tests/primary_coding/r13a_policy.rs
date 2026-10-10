use super::{
    r01::{child, create_job, temp_root},
    r02::event,
    r13a::{reservation, settlement},
};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
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

#[tokio::test]
async fn exact_retry_after_terminal_does_not_reapply_reservation_policy() {
    let (root, store, job) = running("r13a-retry").await;
    let reservation_event = event(
        2,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation(&job.job_id, 2, None)),
        },
    );
    assert_eq!(
        store
            .reserve_coding_budget(reservation_event.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(reservation_event.sequence())
    );
    store
        .append_coding_event(event(
            3,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Completed,
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .reserve_coding_budget(reservation_event)
            .await
            .unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(2).unwrap())
    );
    assert_eq!(
        store
            .reserve_coding_budget(event(
                4,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, 4, None)),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn late_settlement_is_truthful_and_single_final_record() {
    let (root, store, job) = running("r13a-settle").await;
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(&job.job_id, 2, None)),
            },
        ))
        .await
        .unwrap();
    store
        .append_coding_event(event(
            3,
            &job.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Cancelled,
            },
        ))
        .await
        .unwrap();
    let late = event(
        4,
        &job.job_id,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement(&job.job_id, 2, 4)),
        },
    );
    assert_eq!(
        store.settle_coding_budget(late.clone()).await.unwrap(),
        CodingEventAppend::Appended(late.sequence())
    );
    assert_eq!(
        store.settle_coding_budget(late).await.unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(4).unwrap())
    );
    let second = BudgetSettlementV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::new(5).unwrap(),
        BudgetEffectState::Uncertain,
        vec![
            BudgetObservation::new(
                BudgetDimension::ToolCalls,
                None,
                None,
                BudgetObservationValue::Unknown,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(
        store
            .settle_coding_budget(event(
                5,
                &job.job_id,
                None,
                CodingEventKind::BudgetSettled {
                    settlement: Box::new(second),
                },
            ))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn late_child_settlement_uses_original_attribution_after_reconciliation_metadata_changes() {
    let (root, store, job) = running("r13a-late-child").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "r13a-late-child"))
        .await
        .unwrap();
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            Some(&child),
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(&job.job_id, 2, Some(&child))),
            },
        ))
        .await
        .unwrap();
    let db = root.join("state.sqlite3");
    store.close().await;

    // This models durable reconciliation performed by a future owner; it is not
    // a permission bypass for the reservation path under test.
    let raw = pool(&db).await;
    sqlx::query("UPDATE coding_children SET current_invocation_id='invocation-r13a-late-child-new', state='completed' WHERE job_id=? AND agent_id=?")
        .bind(job.job_id.as_str())
        .bind(child.agent_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    sqlx::query("UPDATE coding_jobs SET state='completed' WHERE job_id=?")
        .bind(job.job_id.as_str())
        .execute(&raw)
        .await
        .unwrap();
    raw.close().await;

    let reopened = SqliteStateStore::new(&db);
    reopened
        .settle_coding_budget(event(
            3,
            &job.job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(&job.job_id, 2, 3)),
            },
        ))
        .await
        .unwrap();
    let record = reopened
        .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.reservation.agent_id(), Some(&child.agent_id));
    assert_eq!(
        record.reservation.invocation_id(),
        Some(&child.current_invocation_id)
    );
    assert!(record.settlement.is_some());
    assert_eq!(
        reopened
            .get_coding_job(&job.job_id)
            .await
            .unwrap()
            .unwrap()
            .state,
        JobState::Completed
    );
    let current = reopened
        .list_coding_children(&job.job_id, None, 1)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    assert_eq!(current.state, JobState::Completed);
    assert_ne!(current.current_invocation_id, child.current_invocation_id);
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
