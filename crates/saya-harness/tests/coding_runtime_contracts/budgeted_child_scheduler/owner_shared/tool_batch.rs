use std::time::Duration;

use saya_agent::{EffectAdmission, EffectOutcome, EffectSettlement, LocalStateEffect, ToolEffect};
use saya_harness::coding::budget::ChildCountLimits;
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue};

use crate::scheduler::fixture::{Fixture, limits};

#[tokio::test]
async fn one_tool_batch_has_one_durable_amount_and_dropped_member_stays_unknown() {
    let fixture = Fixture::new("tool-batch-group").await;
    let authority = fixture.add_child("agent-batch", "invocation-batch").await;
    let ledger = fixture
        .ledger(limits(4, 4, 2, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 4,
                tool_calls: 2,
            },
        )
        .await
        .unwrap();
    let child = session.view();
    let effect = ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    };
    let permits = child.admit_tool_batch(&[effect, effect]).await.unwrap();
    assert_eq!(permits.len(), 2);
    let reservations = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 10)
        .await
        .unwrap();
    let tool_rows = reservations
        .items
        .iter()
        .filter(|row| {
            row.reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::ToolCalls)
        })
        .collect::<Vec<_>>();
    assert_eq!(tool_rows.len(), 1);
    assert_eq!(tool_rows[0].reservation.charges()[0].reserved().get(), 2);
    child
        .settle(
            permits[0],
            EffectSettlement::Tool {
                outcome: EffectOutcome::Succeeded,
            },
        )
        .await
        .unwrap();
    let _ = permits[1];
    let before_finish = ledger.snapshot().await;
    assert_eq!(before_finish.tool_calls, 2);
    assert_eq!(before_finish.tool_permits_pending, 1);
    ledger
        .finish_child(session, EffectOutcome::Unknown)
        .await
        .unwrap();

    let after_finish = ledger.snapshot().await;
    assert_eq!(after_finish.tool_calls, 2);
    assert_eq!(after_finish.unknown_tool_calls, 2);
    let tool_row = fixture
        .store
        .get_coding_budget_reservation(
            &fixture.job,
            tool_rows[0].reservation.reservation_sequence(),
        )
        .await
        .unwrap()
        .unwrap();
    let settlement = tool_row.settlement.unwrap();
    assert_eq!(settlement.effect_state(), BudgetEffectState::Uncertain);
    assert_eq!(
        settlement.observations()[0].value(),
        BudgetObservationValue::Unknown
    );
    assert_eq!(after_finish.measured_tool_calls, 1);
    fixture.close().await;
}
