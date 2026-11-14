use std::{fs, num::NonZeroU64};

use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, JobState};

use super::{NOTE, note, reconciled, running_job};
use crate::{
    event_projection_storage::event,
    job_metadata_storage::{create_job, temp_root},
    token_charge_reservations::reservation,
};

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
