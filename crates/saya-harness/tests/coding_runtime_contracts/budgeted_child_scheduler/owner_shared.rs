use std::time::Duration;

use saya_agent::{EffectAdmission, EffectAdmissionError, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::ChildCountLimits;

use crate::scheduler::fixture::{Fixture, limits};

#[path = "owner_shared/physical.rs"]
mod physical;
#[path = "owner_shared/tool_batch.rs"]
mod tool_batch;

#[tokio::test]
async fn root_and_child_share_one_serialized_logical_count_and_model_slot() {
    let fixture = Fixture::new("shared-root-child").await;
    let authority = fixture.add_child("agent-shared", "invocation-shared").await;
    let ledger = fixture
        .ledger(limits(1, 4, 4, 1, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let root = ledger.root_view();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 1,
                tool_calls: 4,
            },
        )
        .await
        .unwrap();
    let child = session.view();
    let (root_result, child_result) =
        tokio::join!(root.admit_model_request(), child.admit_model_request(),);
    assert_eq!(
        (root_result.is_ok() as usize) + (child_result.is_ok() as usize),
        1
    );
    assert_eq!(
        (matches!(&root_result, Err(EffectAdmissionError::Denied)) as usize)
            + (matches!(&child_result, Err(EffectAdmissionError::Denied)) as usize),
        1
    );

    if let Ok(permit) = root_result {
        root.settle(
            permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    }
    if let Ok(permit) = child_result {
        child
            .settle(
                permit,
                EffectSettlement::ModelRequest {
                    outcome: EffectOutcome::NotStarted,
                    reported_usage: None,
                    usage_complete: false,
                },
            )
            .await
            .unwrap();
    }
    ledger
        .finish_child(session, EffectOutcome::NotStarted)
        .await
        .unwrap();
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.logical_requests, 1);
    assert_eq!(snapshot.model_slots_active, 0);
    assert_eq!(snapshot.child_slots_active, 0);
    assert_eq!(
        fixture
            .store
            .list_coding_budget_reservations(&fixture.job, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        2
    );
    fixture.close().await;
}
