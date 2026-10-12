use super::{
    r01::{create_job, temp_root},
    r02::event,
    r13a::reservation,
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{CodingEventKind, JobId, JobState, Sha256Digest};
use sqlx::{Connection, SqliteConnection};
use std::{fs, num::NonZeroU64};

fn reconciled(sequence: u64, job: &JobId, reservation: u64) -> saya_types::CodingEventV1 {
    event(
        sequence,
        job,
        None,
        CodingEventKind::BudgetReconciled {
            reservation_sequence: NonZeroU64::new(reservation).unwrap(),
            note_sha256: Sha256Digest::parse(&"b".repeat(64)).unwrap(),
        },
    )
}

/// A settlement reference that resolves to no recognised event is corruption:
/// recovery counts it unresolved, and reconciliation must not clear it.
#[tokio::test]
async fn a_reservation_with_a_dangling_settlement_reference_is_not_reconcilable() {
    let root = temp_root("r12a-dangling");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "r12a-dangling").await;
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
    for sequence in [2_u64, 3] {
        store
            .reserve_coding_budget(event(
                sequence,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation(&job.job_id, sequence, None)),
                },
            ))
            .await
            .unwrap();
    }
    store.close().await;
    let mut raw = SqliteConnection::connect(&format!("sqlite://{}", db.display()))
        .await
        .unwrap();
    // The schema's foreign key normally forbids this state; corruption is
    // exactly the case where it did not hold, so switch it off to build it.
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut raw)
        .await
        .unwrap();
    sqlx::query("UPDATE coding_budget_reservations SET settled_by_sequence=99 WHERE job_id=? AND reservation_sequence=2")
        .bind(job.job_id.as_str())
        .execute(&mut raw)
        .await
        .unwrap();
    raw.close().await.unwrap();

    let store = SqliteStateStore::new(&db);
    assert_eq!(
        store
            .reconcile_coding_budget(reconciled(4, &job.job_id, 2))
            .await
            .unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store
            .coding_recovery_status(&job.job_id)
            .await
            .unwrap()
            .unresolved_reservations,
        2,
        "the corrupt reference stays unresolved"
    );
    // Positive control: the intact open reservation reconciles.
    store
        .reconcile_coding_budget(reconciled(4, &job.job_id, 3))
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
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
