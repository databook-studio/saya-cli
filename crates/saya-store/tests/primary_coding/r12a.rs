use super::{
    r01::{create_job, temp_root},
    r02::event,
    r13a::reservation,
};
use saya_store::{CodingEventAppend, SqliteStateStore};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetSettlementV1, CodingEventKind, JobId, JobState, Sha256Digest,
};
use std::{fs, num::NonZeroU64};

#[path = "r12a/charges.rs"]
mod charges;
#[path = "r12a/eligibility.rs"]
mod eligibility;
#[path = "r12a/idempotency.rs"]
mod idempotency;

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
