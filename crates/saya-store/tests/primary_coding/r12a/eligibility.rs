use std::{fs, num::NonZeroU64};

use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::CodingEventKind;

use super::{NOTE, note, reconciled, running_job, uncertain_settlement};
use crate::{
    r01::temp_root,
    r02::event,
    r13a::{reservation, settlement},
};

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
