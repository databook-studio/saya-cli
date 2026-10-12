use std::{fs, num::NonZeroU64};

use saya_store::SqliteStateStore;
use saya_types::{BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind};

use super::{NOTE, note, reconciled, running_job};
use crate::{r01::temp_root, r02::event, r13a::charge};

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
