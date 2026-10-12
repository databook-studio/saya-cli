use super::{
    r01::{create_job, temp_root},
    r02::event,
    r13a::{charge, reservation, settlement},
};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobId, JobState, Sha256Digest,
};
use std::{fs, num::NonZeroU64};

const NOTE: &str = "b";

fn note(byte: &str) -> Sha256Digest {
    Sha256Digest::parse(&byte.repeat(64)).unwrap()
}

fn reconciled(
    sequence: u64,
    job: &JobId,
    reservation_sequence: u64,
    note: Sha256Digest,
) -> saya_types::CodingEventV1 {
    event(
        sequence,
        job,
        None,
        CodingEventKind::BudgetReconciled {
            reservation_sequence: NonZeroU64::new(reservation_sequence).unwrap(),
            note_sha256: note,
        },
    )
}

async fn running_job(store: &SqliteStateStore, tag: &str) -> saya_store::CodingJobRecord {
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
    job
}

fn uncertain_settlement(
    job: &JobId,
    reservation_sequence: u64,
    sequence: u64,
) -> BudgetSettlementV1 {
    BudgetSettlementV1::new(
        job.clone(),
        NonZeroU64::new(reservation_sequence).unwrap(),
        NonZeroU64::new(sequence).unwrap(),
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
    .unwrap()
}

#[tokio::test]
async fn open_reservation_stays_unresolved_until_reconciled_then_resume_status_is_clear() {
    let root = temp_root("r12a-open");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r12a-open").await;
    let reserved = reservation(&job.job_id, 2, None);
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reserved.clone()),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .coding_recovery_status(&job.job_id)
            .await
            .unwrap()
            .unresolved_reservations,
        1
    );
    let reconcile = reconciled(3, &job.job_id, 2, note(NOTE));
    assert_eq!(
        store.reconcile_coding_budget(reconcile).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(3).unwrap())
    );
    let status = store.coding_recovery_status(&job.job_id).await.unwrap();
    assert_eq!(status.unresolved_reservations, 0);
    assert_eq!(status.last_sequence, 3);
    let page = store
        .list_coding_budget_reservations(&job.job_id, None, 100)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].reservation, reserved);
    assert_eq!(page.items[0].settlement, None);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn uncertain_settlement_can_be_reconciled_but_a_settled_one_cannot() {
    let root = temp_root("r12a-settled");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r12a-settled").await;
    let uncertain_reservation = reservation(&job.job_id, 2, None);
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(uncertain_reservation.clone()),
            },
        ))
        .await
        .unwrap();
    store
        .settle_coding_budget(event(
            3,
            &job.job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(uncertain_settlement(&job.job_id, 2, 3)),
            },
        ))
        .await
        .unwrap();
    let settled_reservation = reservation(&job.job_id, 4, None);
    store
        .reserve_coding_budget(event(
            4,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(settled_reservation.clone()),
            },
        ))
        .await
        .unwrap();
    store
        .settle_coding_budget(event(
            5,
            &job.job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(&job.job_id, 4, 5)),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        store
            .reconcile_coding_budget(reconciled(6, &job.job_id, 2, note(NOTE)))
            .await
            .unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(6).unwrap())
    );
    assert_eq!(
        store
            .coding_recovery_status(&job.job_id)
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
    assert_eq!(
        store
            .reconcile_coding_budget(reconciled(7, &job.job_id, 4, note(NOTE)))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let page = store
        .list_coding_budget_reservations(&job.job_id, None, 100)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.items[0].reservation, uncertain_reservation);
    assert_eq!(
        page.items[0].settlement,
        Some(uncertain_settlement(&job.job_id, 2, 3))
    );
    assert_eq!(page.items[1].reservation, settled_reservation);
    assert_eq!(
        page.items[1].settlement,
        Some(settlement(&job.job_id, 4, 5))
    );
    assert_eq!(
        store
            .coding_recovery_status(&job.job_id)
            .await
            .unwrap()
            .last_sequence,
        6
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconcile_is_idempotent_and_refuses_a_second_or_foreign_reservation() {
    let root = temp_root("r12a-idempotent");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r12a-idempotent").await;
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
    let first = reconciled(3, &job.job_id, 2, note(NOTE));
    assert_eq!(
        store.reconcile_coding_budget(first.clone()).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(3).unwrap())
    );
    assert_eq!(
        store.reconcile_coding_budget(first).await.unwrap(),
        CodingEventAppend::AlreadyPresent(NonZeroU64::new(3).unwrap())
    );
    assert_eq!(
        store
            .reconcile_coding_budget(reconciled(4, &job.job_id, 2, note("c")))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    let other = create_job(&store, "r12a-foreign").await;
    store
        .append_coding_event(event(
            1,
            &other.job_id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    for sequence in [2_u64, 3] {
        store
            .reserve_coding_budget(event(
                sequence,
                &other.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&other.job_id, sequence, None)),
                },
            ))
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .reconcile_coding_budget(reconciled(5, &job.job_id, 3, note(NOTE)))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn reconciled_reservation_keeps_its_reserved_charges() {
    let root = temp_root("r12a-charges");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = running_job(&store, "r12a-charges").await;
    let charges = vec![
        charge(1),
        BudgetCharge::new(
            BudgetDimension::CommandSlots,
            None,
            None,
            NonZeroU64::new(2).unwrap(),
        )
        .unwrap(),
    ];
    let reserved = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        None,
        None,
        charges,
    )
    .unwrap();
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reserved.clone()),
            },
        ))
        .await
        .unwrap();
    store
        .reconcile_coding_budget(reconciled(3, &job.job_id, 2, note(NOTE)))
        .await
        .unwrap();
    let page = store
        .list_coding_budget_reservations(&job.job_id, None, 100)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].reservation, reserved);
    assert_eq!(page.items[0].reservation.charges().len(), 2);
    assert_eq!(page.items[0].reservation.charges()[0].reserved().get(), 1);
    assert_eq!(page.items[0].reservation.charges()[1].reserved().get(), 2);
    assert_eq!(page.items[0].settlement, None);
    assert_eq!(
        store
            .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
            .await
            .unwrap()
            .unwrap()
            .reservation,
        reserved
    );
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
