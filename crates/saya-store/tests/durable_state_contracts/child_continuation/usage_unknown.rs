//! Unknown model usage (a request Saya gave up on, or one of its attempts)
//! neither refuses a follow-up nor holds back recovery; an unknown effect still
//! does. Both persisted shapes are covered: the current one (`Settled` with an
//! unknown usage observation) and the earlier one (`Uncertain` with only
//! unknown usage observations).
use super::{
    super::event_projection_storage::event,
    support::{Fixture, nz},
};
use saya_types::{
    BudgetCharge, BudgetDimension, BudgetEffectState, BudgetObservation, BudgetObservationValue,
    BudgetReservationV1, BudgetSettlementV1, CodingEventKind,
};

struct Case {
    name: &'static str,
    state: BudgetEffectState,
    unknown: &'static [BudgetDimension],
    refused: bool,
}

const CASES: [Case; 7] = [
    Case {
        name: "current",
        state: BudgetEffectState::Settled,
        unknown: &[BudgetDimension::TransportAttempts],
        refused: false,
    },
    Case {
        name: "previous",
        state: BudgetEffectState::Uncertain,
        unknown: &[BudgetDimension::TransportAttempts],
        refused: false,
    },
    Case {
        name: "tool",
        state: BudgetEffectState::Uncertain,
        unknown: &[BudgetDimension::ToolCalls],
        refused: true,
    },
    Case {
        name: "request-current",
        state: BudgetEffectState::Settled,
        unknown: &[
            BudgetDimension::LogicalRequests,
            BudgetDimension::ModelRequestSlots,
        ],
        refused: false,
    },
    Case {
        name: "request-previous",
        state: BudgetEffectState::Uncertain,
        unknown: &[
            BudgetDimension::LogicalRequests,
            BudgetDimension::ModelRequestSlots,
        ],
        refused: false,
    },
    Case {
        name: "request-beside-tool",
        state: BudgetEffectState::Uncertain,
        unknown: &[
            BudgetDimension::LogicalRequests,
            BudgetDimension::ModelRequestSlots,
            BudgetDimension::ToolCalls,
        ],
        refused: true,
    },
    Case {
        name: "usage-beside-tool",
        state: BudgetEffectState::Uncertain,
        unknown: &[
            BudgetDimension::TransportAttempts,
            BudgetDimension::ToolCalls,
        ],
        refused: true,
    },
];

#[tokio::test]
async fn only_an_unknown_effect_blocks_follow_up_and_resume() {
    for case in CASES {
        let mut f = Fixture::new(&format!("usage-unknown-{}", case.name)).await;
        let charges = case
            .unknown
            .iter()
            .map(|dimension| BudgetCharge::new(*dimension, None, None, nz(1)).unwrap())
            .collect();
        let reservation = BudgetReservationV1::new(
            f.base.job.clone(),
            nz(f.next),
            Some(f.child.agent_id.clone()),
            Some(f.child.current_invocation_id.clone()),
            charges,
        )
        .unwrap();
        let reserved = f.event(CodingEventKind::BudgetReserved {
            reservation: Box::new(reservation),
        });
        let reserved_at = reserved.sequence();
        f.base
            .store
            .reserve_coding_budget_fenced(reserved, Some(&f.authority))
            .await
            .unwrap();
        let observations = case
            .unknown
            .iter()
            .map(|dimension| {
                BudgetObservation::new(*dimension, None, None, BudgetObservationValue::Unknown)
                    .unwrap()
            })
            .collect();
        let settlement = BudgetSettlementV1::new(
            f.base.job.clone(),
            reserved_at,
            nz(f.next),
            case.state,
            observations,
        )
        .unwrap();
        let settled = event(
            f.next,
            &f.base.job,
            None,
            CodingEventKind::BudgetSettled {
                settlement: Box::new(settlement),
            },
        );
        f.next += 1;
        f.base.store.settle_coding_budget(settled).await.unwrap();
        let completion = f.complete().await;

        let unresolved = u64::from(case.refused);
        let status = f
            .base
            .store
            .coding_recovery_status(&f.base.job)
            .await
            .unwrap();
        assert_eq!(status.unresolved_reservations, unresolved, "{}", case.name);
        let listed = f
            .base
            .store
            .list_unresolved_coding_reservations(&f.base.job, None, 10)
            .await
            .unwrap();
        assert_eq!(listed.items.len() as u64, unresolved, "{}", case.name);
        let prepared = f.prepared(completion);
        if case.refused {
            f.assert_refused(prepared).await;
        } else {
            f.base
                .store
                .record_coding_child_invocation(prepared, &f.authority)
                .await
                .unwrap_or_else(|error| panic!("{}: {error:?}", case.name));
        }
        f.base.close().await;
    }
}
