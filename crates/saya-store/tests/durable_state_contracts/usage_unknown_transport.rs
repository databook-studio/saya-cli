//! A transport span whose outcome is unknown pairs with a settlement that
//! records unknown usage, whether that settlement is `Settled` (current) or
//! `Uncertain` (earlier form): neither is a corrupt history.
use super::{
    event_projection_storage::event,
    job_metadata_storage::{create_job, temp_root},
};
use saya_store::SqliteStateStore;
use saya_types::*;
use std::num::NonZeroU64;

fn nz(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

#[tokio::test]
async fn an_unknown_attempt_reads_back_under_either_settlement_shape() {
    for (label, state) in [
        ("current", BudgetEffectState::Settled),
        ("previous", BudgetEffectState::Uncertain),
    ] {
        let root = temp_root(&format!("usage-unknown-transport-{label}"));
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let job = create_job(&store, "usage-unknown-transport").await;
        let id = &job.job_id;
        store
            .append_coding_event(event(
                1,
                id,
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
                BudgetReservationV1::new(id.clone(), nz(seq), None, None, charges).unwrap();
            let kind = CodingEventKind::BudgetReserved {
                reservation: Box::new(reservation),
            };
            store
                .reserve_coding_budget(event(seq, id, None, kind))
                .await
                .unwrap();
        }
        let span = TransportSpanV1::new(TransportSpanData {
            version: 1,
            job_id: id.clone(),
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
            outcome: TransportOutcome::Unknown,
            reported_input_tokens: None,
            reported_output_tokens: None,
            reported_cached_input_tokens: None,
            reported_reasoning_tokens: None,
            reported_cost_micros: None,
            usage_complete: false,
            conversation_correlation_id: None,
            invocation_correlation_id: None,
            actor_correlation_sha256: None,
            request_ordinal: None,
            projection_revision: None,
            projection_size_bytes: None,
            cache_key_digest: None,
            thinking: None,
            streamed: None,
        })
        .unwrap();
        let kind = CodingEventKind::TransportObserved {
            span: Box::new(span),
        };
        store
            .record_coding_transport(event(4, id, None, kind))
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
            BudgetSettlementV1::new(id.clone(), nz(3), nz(5), state, vec![unknown]).unwrap();
        let kind = CodingEventKind::BudgetSettled {
            settlement: Box::new(settlement),
        };
        store
            .settle_coding_budget(event(5, id, None, kind))
            .await
            .unwrap();
        let page = store
            .list_coding_transport_attempts(id, None, 100)
            .await
            .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        assert_eq!(page.items.len(), 1, "{label}");
        let record = &page.items[0];
        assert_eq!(
            record.span.as_ref().map(|span| span.data().outcome),
            Some(TransportOutcome::Unknown),
            "{label}"
        );
        assert_eq!(
            record.settlement.as_ref().map(|s| s.effect_state()),
            Some(state),
            "{label}"
        );
        store.close().await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
