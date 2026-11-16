use super::{
    event_projection_storage::event,
    job_metadata_storage::{child, create_job, temp_root},
};
use saya_store::{CodingBudgetReservationPage, CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobState,
};
use std::{fs, num::NonZeroU64};

pub(super) fn charge(amount: u64) -> BudgetCharge {
    BudgetCharge::new(
        BudgetDimension::ToolCalls,
        None,
        None,
        NonZeroU64::new(amount).unwrap(),
    )
    .unwrap()
}

pub(super) fn reservation(
    job: &saya_types::JobId,
    sequence: u64,
    child: Option<&saya_store::CodingChildRecord>,
) -> BudgetReservationV1 {
    BudgetReservationV1::new(
        job.clone(),
        NonZeroU64::new(sequence).unwrap(),
        child.map(|record| record.agent_id.clone()),
        child.map(|record| record.current_invocation_id.clone()),
        vec![charge(1)],
    )
    .unwrap()
}

pub(super) fn settlement(
    job: &saya_types::JobId,
    reservation_sequence: u64,
    sequence: u64,
) -> BudgetSettlementV1 {
    BudgetSettlementV1::new(
        job.clone(),
        NonZeroU64::new(reservation_sequence).unwrap(),
        NonZeroU64::new(sequence).unwrap(),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::ToolCalls,
                None,
                None,
                BudgetObservationValue::Measured(2),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test]
async fn reservations_and_truthful_overrun_round_trip_on_real_sqlite() {
    let root = temp_root("r13a-roundtrip");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "r13a-roundtrip").await;
    let child = store
        .create_coding_child(child(job.job_id.clone(), "r13a-roundtrip"))
        .await
        .unwrap();
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
    let root_reservation = reservation(&job.job_id, 2, None);
    let child_reservation = reservation(&job.job_id, 3, Some(&child));
    let root_event = event(
        2,
        &job.job_id,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(root_reservation.clone()),
        },
    );
    let child_event = event(
        3,
        &job.job_id,
        Some(&child),
        CodingEventKind::BudgetReserved {
            reservation: Box::new(child_reservation.clone()),
        },
    );
    assert_eq!(
        store
            .reserve_coding_budget(root_event.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(root_event.sequence())
    );
    assert_eq!(
        store.reserve_coding_budget(child_event).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(3).unwrap())
    );
    let settled = settlement(&job.job_id, 2, 4);
    let settlement_event = event(
        4,
        &job.job_id,
        None,
        CodingEventKind::BudgetSettled {
            settlement: Box::new(settled.clone()),
        },
    );
    assert_eq!(
        store.settle_coding_budget(settlement_event).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(4).unwrap())
    );
    store.close().await;

    let reopened = SqliteStateStore::new(&db);
    let stored = reopened
        .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.reservation, root_reservation);
    assert_eq!(stored.settlement, Some(settled));
    let page: CodingBudgetReservationPage = reopened
        .list_coding_budget_reservations(&job.job_id, None, 10)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[1].reservation, child_reservation);
    assert_eq!(
        reopened
            .list_coding_budget_reservations(&job.job_id, None, 0)
            .await
            .unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(
        reopened
            .list_coding_budget_reservations(&job.job_id, None, 101)
            .await
            .unwrap_err(),
        StoreError::LimitExceeded
    );
    assert!(
        reopened
            .get_coding_budget_reservation(
                &saya_types::JobId::parse("job-r13a-unknown").unwrap(),
                NonZeroU64::new(1).unwrap(),
            )
            .await
            .unwrap()
            .is_none()
    );
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
