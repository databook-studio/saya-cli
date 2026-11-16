//! A reservation of any kind keeps a child registered, including one whose
//! only unknown is the usage of a model attempt, in either persisted shape.
use super::{
    event_projection_storage::event,
    job_metadata_storage::{child, create_job, temp_root},
};
use saya_store::{SqliteStateStore, StoreError};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind, JobState,
};
use std::num::NonZeroU64;

fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

#[tokio::test]
async fn a_child_with_an_unknown_usage_attempt_is_never_removed() {
    for (label, state) in [
        ("current", BudgetEffectState::Settled),
        ("previous", BudgetEffectState::Uncertain),
    ] {
        let tag = format!("remove-usage-unknown-{label}");
        let store = SqliteStateStore::new(temp_root(&tag).join("state.sqlite3"));
        let job = create_job(&store, &tag).await;
        let record = store
            .create_coding_child(child(job.job_id.clone(), &tag))
            .await
            .unwrap();
        store
            .append_coding_event(event(
                1,
                &record.job_id,
                None,
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            ))
            .await
            .unwrap();
        let charge =
            BudgetCharge::new(BudgetDimension::TransportAttempts, None, None, nz(1)).unwrap();
        let reservation = BudgetReservationV1::new(
            record.job_id.clone(),
            nz(2),
            Some(record.agent_id.clone()),
            Some(record.current_invocation_id.clone()),
            vec![charge],
        )
        .unwrap();
        let reserved = CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        };
        store
            .reserve_coding_budget(event(2, &record.job_id, Some(&record), reserved))
            .await
            .unwrap();
        let unknown = BudgetObservation::new(
            BudgetDimension::TransportAttempts,
            None,
            None,
            BudgetObservationValue::Unknown,
        )
        .unwrap();
        let settlement =
            BudgetSettlementV1::new(record.job_id.clone(), nz(2), nz(3), state, vec![unknown])
                .unwrap();
        let settled = CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement),
        };
        store
            .settle_coding_budget(event(3, &record.job_id, None, settled))
            .await
            .unwrap();
        let queued = store
            .list_coding_children(&record.job_id, None, 10)
            .await
            .unwrap();
        assert_eq!(
            queued.items[0].state,
            JobState::Queued,
            "{label}: only the reservation can refuse the removal"
        );

        let result = store
            .remove_unscheduled_coding_child(
                &record.job_id,
                &record.agent_id,
                &record.current_invocation_id,
            )
            .await;

        assert!(
            matches!(result, Err(StoreError::Conflict)),
            "{label}: {result:?}"
        );
        let page = store
            .list_coding_children(&record.job_id, None, 10)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1, "{label}");
    }
}
