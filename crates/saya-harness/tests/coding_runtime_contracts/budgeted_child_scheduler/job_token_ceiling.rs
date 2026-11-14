use super::fixture::{Fixture, limits};
use saya_agent::{
    EffectAdmission, EffectAdmissionError, EffectCallKind, EffectOutcome, EffectPermit,
    EffectSettlement, PhysicalAttempt, TokenUsage,
};
use saya_harness::coding::budget::{ChildCountLimits, JobBudgetLedger, RootBudgetView};
use std::time::Duration;

fn job_limits() -> saya_harness::coding::budget::CountLimits {
    limits(10, 10, 10, 4, 3, Duration::from_secs(30))
}

fn denied(result: Result<EffectPermit, EffectAdmissionError>) {
    assert!(
        matches!(result, Err(EffectAdmissionError::Denied)),
        "the job-wide reported-token ceiling must deny the model request"
    );
}

/// Admits one physical send on a fresh model permit and settles it with the
/// given provider-reported usage.
async fn settle_reported(
    root: &RootBudgetView,
    model: EffectPermit,
    input: u64,
    output: u64,
    complete: bool,
) {
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let permit = root.admit_physical_attempt(attempt).await.unwrap();
    root.settle(
        permit,
        EffectSettlement::PhysicalAttempt {
            attempt,
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(TokenUsage::new(input, output)),
            usage_complete: complete,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn without_a_ceiling_a_million_reported_tokens_still_admit() {
    let fixture = Fixture::new("ceiling-none").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    let root = ledger.root_view();
    let model = root.admit_model_request().await.unwrap();
    settle_reported(&root, model, 500_000, 500_000, true).await;
    root.admit_model_request()
        .await
        .expect("the default has no ceiling, so reported usage never denies admission");
    fixture.close().await;
}

#[tokio::test]
async fn a_ceiling_denies_once_reported_usage_passes_it() {
    let fixture = Fixture::new("ceiling-passed").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(Some(100)).await;
    let root = ledger.root_view();
    root.admit_model_request()
        .await
        .expect("zero reported tokens admit");
    let first = root.admit_model_request().await.unwrap();
    settle_reported(&root, first, 40, 40, true).await;
    root.admit_model_request()
        .await
        .expect("80 reported tokens stay below the 100-token ceiling");
    let second = root.admit_model_request().await.unwrap();
    settle_reported(&root, second, 20, 20, true).await;
    denied(root.admit_model_request().await);
    fixture.close().await;
}

#[tokio::test]
async fn a_ceiling_denies_exactly_at_the_limit() {
    let fixture = Fixture::new("ceiling-boundary").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(Some(100)).await;
    let root = ledger.root_view();
    let first = root.admit_model_request().await.unwrap();
    settle_reported(&root, first, 40, 40, true).await;
    let second = root
        .admit_model_request()
        .await
        .expect("80 reported tokens stay below the 100-token ceiling");
    settle_reported(&root, second, 10, 10, true).await;
    denied(root.admit_model_request().await);
    fixture.close().await;
}

#[tokio::test]
async fn incomplete_or_unknown_usage_does_not_count_towards_the_ceiling() {
    let fixture = Fixture::new("ceiling-incomplete").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(Some(100)).await;
    let root = ledger.root_view();
    let first = root.admit_model_request().await.unwrap();
    settle_reported(&root, first, 500, 500, false).await;
    let second = root
        .admit_model_request()
        .await
        .expect("an incomplete report is an estimate, not reported usage");
    let attempt = PhysicalAttempt::new(second, EffectCallKind::Answer, 0);
    let permit = root.admit_physical_attempt(attempt).await.unwrap();
    root.settle(
        permit,
        EffectSettlement::PhysicalAttempt {
            attempt,
            outcome: EffectOutcome::Unknown,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    root.admit_model_request()
        .await
        .expect("unknown usage is not reported usage, so the ceiling never sees it");
    fixture.close().await;
}

#[tokio::test]
async fn the_ceiling_is_runtime_configuration_not_durable_policy() {
    let fixture = Fixture::new("ceiling-runtime-only").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(Some(100)).await;
    let root = ledger.root_view();
    let first = root.admit_model_request().await.unwrap();
    settle_reported(&root, first, 60, 60, true).await;
    denied(root.admit_model_request().await);
    // Rebuilding the owner for the same job with equal limits proves the
    // ceiling was never folded into the durable policy or its counts: the
    // resume authenticates the proposed limits against the stored policy.
    let rebuilt = JobBudgetLedger::resume(fixture.store.clone(), fixture.job.clone(), job_limits())
        .await
        .expect("an equal limits proposal must still match the recorded policy");
    rebuilt
        .root_view()
        .admit_model_request()
        .await
        .expect("a rebuilt owner carries no ceiling: it is runtime configuration only");
    fixture.close().await;
}

#[tokio::test]
async fn a_child_view_is_denied_by_the_job_wide_ceiling() {
    let fixture = Fixture::new("ceiling-child").await;
    let ledger = fixture.ledger(job_limits()).await.unwrap();
    ledger.set_reported_token_ceiling(Some(100)).await;
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
    let model = root.admit_model_request().await.unwrap();
    settle_reported(&root, model, 60, 60, true).await;
    denied(child.view().admit_model_request().await);
    fixture.close().await;
}
