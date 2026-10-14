use super::fixture::{Fixture, limits};
use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, PhysicalAttempt,
    PhysicalRequestMetadata, TokenUsage,
};
use saya_harness::coding::budget::{ChildCountLimits, JobBudgetLedger};
use saya_types::{Sha256Digest, TokenBudgetPolicyData, TokenBudgetPolicyV1};
use std::time::Duration;
fn policy() -> TokenBudgetPolicyV1 {
    TokenBudgetPolicyV1::new(TokenBudgetPolicyData {
        version: 1,
        total_tokens: 24,
        provider: "openai".into(),
        model: "verified-fixture".into(),
        endpoint_sha256: TokenBudgetPolicyV1::endpoint_digest(
            "http://127.0.0.1:1/v1/chat/completions",
        )
        .unwrap(),
        verification_sha256: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        input_context_upper_bound: 8,
        output_tokens: 4,
    })
    .unwrap()
}
fn request() -> PhysicalRequestMetadata {
    PhysicalRequestMetadata::new(
        "openai",
        "verified-fixture",
        "http://127.0.0.1:1/v1/chat/completions",
        4,
    )
    .unwrap()
}
#[tokio::test]
async fn root_child_and_retry_share_conservative_tokens_and_unknown_survives_restart() {
    let fixture = Fixture::new("token-shared").await;
    let limits = limits(10, 10, 10, 4, 3, Duration::from_secs(30));
    let ledger = fixture.ledger(limits).await.unwrap();
    ledger.configure_token_budget(policy()).await.unwrap();
    let authority = fixture.add_child("agent-a", "invocation-a").await;
    let child = ledger
        .begin_child(
            authority.agent_id(),
            authority.invocation_id(),
            ChildCountLimits {
                logical_requests: 5,
                tool_calls: 5,
            },
        )
        .await
        .unwrap();
    let root = ledger.root_view();
    let root_model = root.admit_model_request().await.unwrap();
    let child_model = child.view().admit_model_request().await.unwrap();
    let root_attempt = PhysicalAttempt::new(root_model, EffectCallKind::Answer, 0);
    let child_attempt = PhysicalAttempt::new(child_model, EffectCallKind::Answer, 0);
    assert!(
        root.admit_physical_attempt(root_attempt).await.is_err(),
        "strict request metadata is mandatory"
    );
    let first = root
        .admit_physical_request(root_attempt, Some(&request()))
        .await
        .unwrap();
    let second = child
        .view()
        .admit_physical_request(child_attempt, Some(&request()))
        .await
        .unwrap();
    assert_eq!(ledger.snapshot().await.tokens_charged, Some(24));
    root.settle(
        first,
        EffectSettlement::PhysicalAttempt {
            attempt: root_attempt,
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(TokenUsage::new(2, 3)),
            usage_complete: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.tokens_charged, Some(17));
    child
        .view()
        .settle(
            second,
            EffectSettlement::PhysicalAttempt {
                attempt: child_attempt,
                outcome: EffectOutcome::Unknown,
                reported_usage: None,
                usage_complete: false,
            },
        )
        .await
        .unwrap();
    let retry = PhysicalAttempt::new(root_model, EffectCallKind::Answer, 1);
    assert!(
        root.admit_physical_request(retry, Some(&request()))
            .await
            .is_err(),
        "only seven tokens remain; retry needs twelve"
    );
    let restored = JobBudgetLedger::resume(fixture.store.clone(), fixture.job.clone(), limits)
        .await
        .unwrap();
    assert_eq!(restored.snapshot().await.tokens_charged, Some(17));
    fixture.close().await;
}
#[tokio::test]
async fn reported_bound_violation_preserves_actual_usage_and_fences_admission() {
    let fixture = Fixture::new("token-overrun").await;
    let ledger = fixture
        .ledger(limits(10, 10, 10, 4, 3, Duration::from_secs(30)))
        .await
        .unwrap();
    ledger.configure_token_budget(policy()).await.unwrap();
    let root = ledger.root_view();
    let model = root.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let permit = root
        .admit_physical_request(attempt, Some(&request()))
        .await
        .unwrap();
    assert!(
        root.settle(
            permit,
            EffectSettlement::PhysicalAttempt {
                attempt,
                outcome: EffectOutcome::Succeeded,
                reported_usage: Some(TokenUsage::new(40, 3)),
                usage_complete: true
            }
        )
        .await
        .is_err()
    );
    let snapshot = ledger.snapshot().await;
    assert_eq!(snapshot.tokens_charged, Some(43));
    assert!(snapshot.token_bound_violated);
    assert!(root.admit_model_request().await.is_err());
    let records = fixture
        .store
        .list_coding_budget_reservations(&fixture.job, None, 100)
        .await
        .unwrap();
    assert!(
        records
            .items
            .iter()
            .filter_map(|record| record.settlement.as_ref())
            .flat_map(|record| record.observations())
            .any(|observation| observation.value()
                == saya_types::BudgetObservationValue::Measured(40))
    );
    fixture.close().await;
}
