use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, PhysicalAttempt,
    PhysicalRequestMetadata, TokenUsage,
};
use saya_harness::coding::budget::JobBudgetLedger;
use saya_types::{Sha256Digest, TokenBudgetPolicyData, TokenBudgetPolicyV1};

use super::support::Fixture;

fn policy() -> TokenBudgetPolicyV1 {
    TokenBudgetPolicyV1::new(TokenBudgetPolicyData {
        version: 1,
        total_tokens: 12,
        provider: "openai".into(),
        model: "uncapped-token-fixture".into(),
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
        "uncapped-token-fixture",
        "http://127.0.0.1:1/v1/chat/completions",
        Some(4),
    )
    .unwrap()
}

#[tokio::test]
async fn uncapped_mode_keeps_independent_verified_token_policy_binding() {
    let fixture = Fixture::new("uncapped-verified-token-policy").await;
    let ledger = JobBudgetLedger::new_uncapped(fixture.store.clone(), fixture.job.clone(), 2, 2)
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
    root.settle(
        permit,
        EffectSettlement::PhysicalAttempt {
            attempt,
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(TokenUsage::new(8, 4)),
            usage_complete: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(ledger.snapshot().await.token_limit, Some(12));
    let retry = PhysicalAttempt::new(model, EffectCallKind::Answer, 1);
    assert!(
        root.admit_physical_request(retry, Some(&request()))
            .await
            .is_err(),
        "uncapped aggregate mode must keep the separate verified token policy"
    );
    fixture.close().await;
}
