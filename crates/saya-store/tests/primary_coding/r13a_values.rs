use super::{
    r01::{create_job, temp_root},
    r02::event,
};
use saya_store::SqliteStateStore;
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobState,
};
use std::{fs, num::NonZeroU64};

#[tokio::test]
async fn typed_zero_estimated_unknown_unavailable_and_uncertain_values_persist_verbatim() {
    let root = temp_root("r13a-values");
    let db = root.join("state.sqlite3");
    let store = SqliteStateStore::new(&db);
    let job = create_job(&store, "r13a-values").await;
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
    let dimensions = [
        BudgetDimension::ToolCalls,
        BudgetDimension::LogicalRequests,
        BudgetDimension::TransportAttempts,
        BudgetDimension::ElapsedMilliseconds,
    ];
    let reservation = BudgetReservationV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        None,
        None,
        dimensions
            .iter()
            .map(|dimension| {
                BudgetCharge::new(*dimension, None, None, NonZeroU64::new(1).unwrap()).unwrap()
            })
            .collect(),
    )
    .unwrap();
    let observations = vec![
        BudgetObservation::new(
            BudgetDimension::ToolCalls,
            None,
            None,
            BudgetObservationValue::Measured(0),
        )
        .unwrap(),
        BudgetObservation::new(
            BudgetDimension::LogicalRequests,
            None,
            None,
            BudgetObservationValue::Estimated(9),
        )
        .unwrap(),
        BudgetObservation::new(
            BudgetDimension::TransportAttempts,
            None,
            None,
            BudgetObservationValue::Unknown,
        )
        .unwrap(),
        BudgetObservation::new(
            BudgetDimension::ElapsedMilliseconds,
            None,
            None,
            BudgetObservationValue::Unavailable,
        )
        .unwrap(),
    ];
    let settlement = BudgetSettlementV1::new(
        job.job_id.clone(),
        NonZeroU64::new(2).unwrap(),
        NonZeroU64::new(3).unwrap(),
        BudgetEffectState::Uncertain,
        observations.clone(),
    )
    .unwrap();
    store
        .reserve_coding_budget(event(
            2,
            &job.job_id,
            None,
            CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation),
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
                settlement: Box::new(settlement),
            },
        ))
        .await
        .unwrap();
    store.close().await;
    let reopened = SqliteStateStore::new(&db);
    let persisted = reopened
        .get_coding_budget_reservation(&job.job_id, NonZeroU64::new(2).unwrap())
        .await
        .unwrap()
        .unwrap()
        .settlement
        .unwrap();
    assert_eq!(persisted.effect_state(), BudgetEffectState::Uncertain);
    assert_eq!(persisted.observations(), observations);
    reopened.close().await;
    let _ = fs::remove_dir_all(root);
}
