use super::{
    r01::{create_job, temp_root},
    r02::event,
    r13a::{reservation, settlement},
};
use saya_store::SqliteStateStore;
use saya_types::{CodingEventKind, JobId, JobState, Sha256Digest};
use std::{fs, num::NonZeroU64};

fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

async fn reserve(store: &SqliteStateStore, job: &JobId, sequence: u64) {
    store
        .reserve_coding_budget(event(
            sequence,
            job,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation(job, sequence, None)),
            },
        ))
        .await
        .unwrap();
}

/// Reservations 2 (open), 4 (settled), 6 (open, then reconciled), 8 (open).
#[tokio::test]
async fn unresolved_listing_is_open_and_unreconciled_oldest_first_and_pages() {
    let root = temp_root("r12a-list");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "r12a-list").await;
    let id = job.job_id;
    store
        .append_coding_event(event(
            1,
            &id,
            None,
            CodingEventKind::StateChanged {
                state: JobState::Running,
            },
        ))
        .await
        .unwrap();
    reserve(&store, &id, 2).await;
    reserve(&store, &id, 3).await;
    store
        .settle_coding_budget(event(
            4,
            &id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement(&id, 3, 4)),
            },
        ))
        .await
        .unwrap();
    reserve(&store, &id, 5).await;
    store
        .reconcile_coding_budget(event(
            6,
            &id,
            None,
            CodingEventKind::BudgetReconciled {
                reservation_sequence: nz(5),
                note_sha256: Sha256Digest::parse(&"b".repeat(64)).unwrap(),
            },
        ))
        .await
        .unwrap();
    reserve(&store, &id, 7).await;

    let all = store
        .list_unresolved_coding_reservations(&id, None, 10)
        .await
        .unwrap();
    let listed: Vec<u64> = all
        .items
        .iter()
        .map(|item| item.reservation.reservation_sequence().get())
        .collect();
    assert_eq!(
        listed,
        vec![2, 7],
        "settled (3) and reconciled (5) are excluded"
    );
    assert_eq!(all.next_cursor, None);
    assert_eq!(
        store
            .coding_recovery_status(&id)
            .await
            .unwrap()
            .unresolved_reservations,
        2,
        "the listing and the recovery count use one predicate"
    );

    let first = store
        .list_unresolved_coding_reservations(&id, None, 1)
        .await
        .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.next_cursor, Some(nz(2)));
    let second = store
        .list_unresolved_coding_reservations(&id, first.next_cursor, 1)
        .await
        .unwrap();
    assert_eq!(second.items[0].reservation.reservation_sequence(), nz(7));
    assert_eq!(second.next_cursor, None);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
