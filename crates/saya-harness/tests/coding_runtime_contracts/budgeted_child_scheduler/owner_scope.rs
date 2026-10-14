use std::time::Duration;

use saya_agent::{EffectAdmission, EffectAdmissionError, EffectOutcome, EffectSettlement};
use saya_harness::coding::budget::ChildCountLimits;

use super::fixture::{Fixture, limits};

#[tokio::test]
async fn child_scope_tightens_without_reducing_root_shared_allowance() {
    let fixture = Fixture::new("local-count-scope").await;
    let authority = fixture.add_child("agent-local", "invocation-local").await;
    let ledger = fixture
        .ledger(limits(4, 4, 4, 2, 1, Duration::from_secs(5)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 1,
                tool_calls: 2,
            },
        )
        .await
        .unwrap();
    let child = session.view();
    let child_permit = child.admit_model_request().await.unwrap();
    child
        .settle(
            child_permit,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::NotStarted,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        child.admit_model_request().await,
        Err(EffectAdmissionError::Denied)
    ));

    let root = ledger.root_view();
    let root_permit = root.admit_model_request().await.unwrap();
    root.settle(
        root_permit,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::NotStarted,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    ledger
        .finish_child(session, EffectOutcome::Succeeded)
        .await
        .unwrap();
    assert_eq!(ledger.snapshot().await.logical_requests, 2);
    fixture.close().await;
}
