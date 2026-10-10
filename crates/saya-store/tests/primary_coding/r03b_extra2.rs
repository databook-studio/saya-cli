//! The first-policy insert guard: a Running job with existing budget
//! history and no policy row must be refused on the insert path so
//! unauthenticated history can never become resumable.

use super::r01::temp_root;
use super::r03b::{policy, running_job};
use saya_store::{CodingEventAppend, SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetReservationV1, CodingEventKind, CodingEventV1,
};
use std::{fs, num::NonZeroU64};

/// A Running job that already has a budget reservation but no policy row
/// must be refused by `record_coding_budget_policy` without inserting: the
/// harness records a first policy before its fresh-job guard, so an insert
/// here would let later `resume` calls adopt history that was never
/// authenticated by a policy. The refusal is typed (`StoreError::Conflict`)
/// and leaves no policy row behind.
#[tokio::test]
async fn first_policy_is_refused_when_budget_history_exists_and_leaves_no_row() {
    let root = temp_root("r03bx-first-policy-history");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = running_job(&store, "r03bx-first-policy-history").await;

    // Budget history written through the public store API while the job is
    // Running: sequence 2, right after the running event, with no policy
    // row recorded first.
    let reservation = BudgetReservationV1::new(
        job.clone(),
        NonZeroU64::new(2).unwrap(),
        None,
        None,
        vec![
            BudgetCharge::new(
                BudgetDimension::LogicalRequests,
                None,
                None,
                NonZeroU64::new(1).unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let event = CodingEventV1::new(
        NonZeroU64::new(2).unwrap(),
        job.clone(),
        None,
        None,
        CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        },
    )
    .unwrap();
    assert_eq!(
        store.reserve_coding_budget(event).await.unwrap(),
        CodingEventAppend::Appended(NonZeroU64::new(2).unwrap())
    );
    assert!(
        store
            .get_coding_budget_policy(&job)
            .await
            .unwrap()
            .is_none()
    );

    // The first-policy insert must be refused, not adopted.
    let refused = store
        .record_coding_budget_policy(&job, &policy(120))
        .await
        .unwrap_err();
    assert_eq!(refused, StoreError::Conflict);

    // No policy row was inserted and the history is still readable.
    assert!(
        store
            .get_coding_budget_policy(&job)
            .await
            .unwrap()
            .is_none()
    );
    let listed = store
        .list_coding_budget_reservations(&job, None, 100)
        .await
        .unwrap();
    assert_eq!(listed.items.len(), 1);
    store.close().await;
    let _ = fs::remove_dir_all(root);
}
