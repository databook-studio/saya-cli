//! Unknown model-attempt usage never decides whether a finished child's effects
//! are settled; an unknown tool effect still does, with or without a cut attempt.
use std::time::Duration;

use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, LocalStateEffect,
    PhysicalAttempt, TokenUsage, ToolEffect,
};
use saya_harness::coding::budget::{ChildCountLimits, JobBudgetLedger};

use super::fixture::{Fixture, limits};

/// Runs one child whose first transport attempt is cut and whose retry
/// succeeds, optionally followed by one tool call settling as `tool`.
async fn finished_child(fixture: &Fixture, tool: Option<EffectOutcome>) -> (JobBudgetLedger, bool) {
    let authority = fixture.add_child("agent-cut", "invocation-cut").await;
    let ledger = fixture
        .ledger(limits(4, 8, 4, 2, 1, Duration::from_secs(30)))
        .await
        .unwrap();
    let session = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 2,
                tool_calls: 2,
            },
        )
        .await
        .unwrap();
    let child = session.view().clone();
    let model = child.admit_model_request().await.unwrap();
    let cut = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let send = child.admit_physical_attempt(cut).await.unwrap();
    child
        .settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt: cut,
                outcome: EffectOutcome::Unknown,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    let retry = PhysicalAttempt::new(model, EffectCallKind::Answer, 1);
    let send = child.admit_physical_attempt(retry).await.unwrap();
    child
        .settle(
            send,
            EffectSettlement::PhysicalAttempt {
                attempt: retry,
                outcome: EffectOutcome::Succeeded,
                reported_usage: Some(TokenUsage::new(10, 20)),
                usage_complete: true,
            },
        )
        .await
        .unwrap();
    child
        .settle(
            model,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::Succeeded,
                reported_usage: Some(TokenUsage::new(10, 20)),
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    if let Some(outcome) = tool {
        let effect = ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        };
        let permits = child.admit_tool_batch(&[effect]).await.unwrap();
        child
            .settle(permits[0], EffectSettlement::Tool { outcome })
            .await
            .unwrap();
    }
    ledger
        .finish_child(session, EffectOutcome::Succeeded)
        .await
        .unwrap();
    let started = ledger
        .child_started(authority.agent_id(), authority.invocation_id())
        .await;
    (ledger, started)
}

#[tokio::test]
async fn a_cut_attempt_alone_leaves_a_finished_child_settled() {
    let fixture = Fixture::new("usage-unknown-alone").await;
    let (ledger, unsettled) = finished_child(&fixture, None).await;
    assert!(!unsettled, "unknown usage is not an unsettled child");
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    assert_eq!(snapshot.unknown_tool_calls, 0);
    assert!(!snapshot.poisoned);
    fixture.close().await;
}

#[tokio::test]
async fn a_cut_attempt_never_masks_an_unknown_tool_effect() {
    let fixture = Fixture::new("usage-unknown-tool").await;
    let (ledger, unsettled) = finished_child(&fixture, Some(EffectOutcome::Unknown)).await;
    assert!(
        unsettled,
        "an unknown tool effect still leaves the child unsettled"
    );
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.unknown_tool_calls, 1);
    assert_eq!(snapshot.unknown_transport_attempts, 1);
    fixture.close().await;
}
