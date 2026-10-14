use super::{
    event_projection_storage::event,
    job_metadata_storage::{create_job, temp_root},
};
use saya_store::{CodingEventAppend, SqliteStateStore};
use saya_types::*;
use std::num::NonZeroU64;
fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}
#[tokio::test]
async fn transport_history_preserves_missing_observation_until_real_source_records_arrive() {
    let root = temp_root("transport-history");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let job = create_job(&store, "transport").await;
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
    for (seq, dimensions) in [
        (
            2,
            vec![
                BudgetDimension::LogicalRequests,
                BudgetDimension::ModelRequestSlots,
            ],
        ),
        (3, vec![BudgetDimension::TransportAttempts]),
    ] {
        let charges = dimensions
            .into_iter()
            .map(|d| BudgetCharge::new(d, None, None, nz(1)).unwrap())
            .collect();
        let reservation =
            BudgetReservationV1::new(job.job_id.clone(), nz(seq), None, None, charges).unwrap();
        store
            .reserve_coding_budget(event(
                seq,
                &job.job_id,
                None,
                CodingEventKind::BudgetReserved {
                    reservation: Box::new(reservation),
                },
            ))
            .await
            .unwrap();
    }
    let before = store
        .list_coding_transport_attempts(&job.job_id, None, 100)
        .await
        .unwrap();
    assert_eq!(before.items.len(), 1);
    assert!(before.items[0].span.is_none());
    assert!(!before.items[0].timing_complete());
    let span = TransportSpanV1::new(TransportSpanData {
        version: 1,
        job_id: job.job_id.clone(),
        agent_id: None,
        invocation_id: None,
        runtime_epoch: None,
        reservation_sequence: nz(3),
        parent_reservation_sequence: nz(2),
        ordinal: 0,
        call_kind: TransportCallKind::Answer,
        process_id: 10,
        clock_domain: "p10-t100".into(),
        dispatch_started_ns: Some(12),
        finished_ns: 33,
        outcome: TransportOutcome::Completed,
        reported_input_tokens: Some(2),
        reported_output_tokens: Some(3),
        usage_complete: true,
    })
    .unwrap();
    let observed = event(
        4,
        &job.job_id,
        None,
        CodingEventKind::TransportObserved {
            span: Box::new(span.clone()),
        },
    );
    assert_eq!(
        store
            .record_coding_transport(observed.clone())
            .await
            .unwrap(),
        CodingEventAppend::Appended(nz(4))
    );
    assert_eq!(
        store.record_coding_transport(observed).await.unwrap(),
        CodingEventAppend::AlreadyPresent(nz(4))
    );
    let middle = store
        .list_coding_transport_attempts(&job.job_id, None, 1)
        .await
        .unwrap();
    assert_eq!(middle.items[0].span, Some(span));
    assert!(!middle.items[0].timing_complete());
    let settlement = BudgetSettlementV1::new(
        job.job_id.clone(),
        nz(3),
        nz(5),
        BudgetEffectState::Settled,
        vec![
            BudgetObservation::new(
                BudgetDimension::TransportAttempts,
                None,
                None,
                BudgetObservationValue::Measured(1),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    store
        .settle_coding_budget(event(
            5,
            &job.job_id,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement),
            },
        ))
        .await
        .unwrap();
    store.close().await;
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let after = store
        .list_coding_transport_attempts(&job.job_id, None, 100)
        .await
        .unwrap();
    assert!(after.items[0].timing_complete());
    assert_eq!(after.items[0].span.as_ref().unwrap().data().finished_ns, 33);
    store.close().await;
    std::fs::remove_dir_all(root).unwrap();
}
