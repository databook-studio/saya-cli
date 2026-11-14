use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_config::{BudgetLimit, CodingBudgetMode, ResolvedJobLimits};
use saya_types::JobId;
use std::{sync::atomic::Ordering, time::Duration};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

/// The ceilings the mode decides between: a bounded job gets the numbers, and
/// an uncapped one gets `Unlimited` on every dimension — the state the
/// resolver produces for `limits.job.budget_mode = "uncapped"`.
fn job_limits(mode: CodingBudgetMode) -> ResolvedJobLimits {
    if mode == CodingBudgetMode::Uncapped {
        return ResolvedJobLimits {
            budget_mode: mode,
            logical_requests: BudgetLimit::Unlimited,
            transport_attempts: BudgetLimit::Unlimited,
            tool_calls: BudgetLimit::Unlimited,
            known_tokens: BudgetLimit::Unlimited,
            job_tokens: BudgetLimit::Unlimited,
            elapsed_seconds: BudgetLimit::Unlimited,
            ..ResolvedJobLimits::default()
        };
    }
    // Exercise the logical ceiling beyond the fixed boundary.
    ResolvedJobLimits {
        budget_mode: mode,
        logical_requests: finite(80),
        transport_attempts: finite(160),
        tool_calls: finite(100),
        known_tokens: finite(100_000),
        job_tokens: BudgetLimit::Unlimited,
        elapsed_seconds: finite(300),
        ..ResolvedJobLimits::default()
    }
}

async fn run_repeated_status(label: &str, mode: CodingBudgetMode) -> (usize, bool, String) {
    let provider = Provider::repeated_status();
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job = job_limits(mode);
    // This scenario measures the request ceiling, not the progress guards.
    runtime.resolved.limits.job.no_progress_turns = 0;
    runtime.resolved.limits.job.repeat_limit = 0;
    let application = CodingApplication::open(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Read status repeatedly without changing files".into(),
        vec![],
    )
    .unwrap();
    let proposal = application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(120), job.run(&NoopEventSink))
        .await
        .expect("the scripted read-only loop terminates");
    let requests = provider.calls.load(Ordering::Acquire);
    let complete = result.as_ref().is_ok_and(|output| !output.truncated);
    let outcome = result.map_or_else(|error| error.to_string(), |_| "complete".to_owned());
    (requests, complete, outcome)
}

#[tokio::test]
async fn uncapped_mode_crosses_eighty_logical_requests_while_bounded_stops_at_eighty() {
    let (bounded_requests, bounded_complete, bounded_outcome) =
        run_repeated_status("bounded-eighty-status", CodingBudgetMode::Bounded).await;
    assert_eq!(
        bounded_requests, 80,
        "the control run must actually hit the configured boundary: {bounded_outcome}"
    );
    assert!(
        !bounded_complete,
        "bounded mode stops before its 81st request"
    );

    let (uncapped_requests, uncapped_complete, uncapped_outcome) =
        run_repeated_status("uncapped-eighty-status", CodingBudgetMode::Uncapped).await;
    assert_eq!(uncapped_requests, 81, "{uncapped_outcome}");
    assert!(uncapped_complete, "{uncapped_outcome}");
}
