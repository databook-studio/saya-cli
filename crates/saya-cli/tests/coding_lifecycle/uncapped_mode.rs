use super::{
    provider::Provider,
    support::{EVENT_WAIT, Fixture},
};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_config::{BudgetLimit, CodingBudgetMode};
use saya_types::JobId;
use std::{sync::atomic::Ordering, time::Duration};

fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}

#[derive(Clone, Copy)]
enum AggregateCap {
    LogicalRequests,
    PhysicalAttempts,
    ToolCalls,
    PerLoopTokens,
    JobTokens,
}

impl AggregateCap {
    fn label(self) -> &'static str {
        match self {
            Self::LogicalRequests => "logical-requests",
            Self::PhysicalAttempts => "physical-attempts",
            Self::ToolCalls => "tool-calls",
            Self::PerLoopTokens => "per-loop-tokens",
            Self::JobTokens => "job-tokens",
        }
    }
}

async fn run_with_uncapped_mode(
    label: &str,
    provider: &Provider,
    cap: AggregateCap,
) -> (usize, bool, String) {
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let mut runtime = fixture.runtime.clone();
    let job_limits = &mut runtime.resolved.limits.job;
    job_limits.budget_mode = CodingBudgetMode::Uncapped;
    // These direct resolved values make the owner red fast. No config can
    // state them: `[limits.job]` refuses a dimension beside `uncapped`.
    // Written here anyway, so an adapter that reads a dimension without
    // consulting the mode fails this test.
    match cap {
        AggregateCap::LogicalRequests => job_limits.logical_requests = finite(1),
        AggregateCap::PhysicalAttempts => job_limits.transport_attempts = finite(1),
        AggregateCap::ToolCalls => job_limits.tool_calls = finite(1),
        AggregateCap::PerLoopTokens => job_limits.known_tokens = finite(1),
        AggregateCap::JobTokens => job_limits.job_tokens = finite(1),
    }
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
        "Fix the one allowed file".into(),
        vec!["only.txt".into()],
    )
    .unwrap();
    let proposal = application.propose(&request).await.unwrap();
    assert_eq!(proposal.request()["budget_mode"], "uncapped");
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let result = tokio::time::timeout(EVENT_WAIT, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates instead of hanging");
    let requests = provider.calls.load(Ordering::Acquire);
    let policy = fixture
        .store
        .get_coding_budget_policy(&JobId::parse(label).unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        policy.policy,
        saya_types::CodingBudgetPolicy::UncappedV2(_)
    ));
    let exported = application
        .export(&JobId::parse(label).unwrap(), None, None)
        .await
        .unwrap();
    assert_eq!(exported["budget_mode"], "uncapped");
    assert_eq!(
        exported["budget_policy"]["limits"]["logical_requests"],
        serde_json::Value::Null
    );
    let complete = result.as_ref().is_ok_and(|output| !output.truncated);
    let outcome = result.map_or_else(|error| error.to_string(), |_| "complete".to_owned());
    (requests, complete, outcome)
}

#[tokio::test]
async fn uncapped_mode_crosses_each_configured_aggregate_ceiling() {
    let mut failures = Vec::new();
    for cap in [
        AggregateCap::LogicalRequests,
        AggregateCap::PhysicalAttempts,
        AggregateCap::ToolCalls,
        AggregateCap::PerLoopTokens,
        AggregateCap::JobTokens,
    ] {
        let provider = Provider::one_path();
        let label = format!("uncapped-{}", cap.label());
        let (requests, complete, outcome) = run_with_uncapped_mode(&label, &provider, cap).await;
        if requests <= 1 || !complete {
            failures.push(format!(
                "{}: requests={requests}, complete={complete}, outcome={outcome}",
                cap.label()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "uncapped mode still enforced configured aggregate limits: {failures:?}"
    );
}

#[tokio::test]
async fn uncapped_mode_does_not_cancel_at_the_configured_elapsed_deadline() {
    let provider = Provider::parity();
    let fixture = Fixture::with_endpoint("uncapped-elapsed", Some(&provider.endpoint)).await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job.budget_mode = CodingBudgetMode::Uncapped;
    runtime.resolved.limits.job.elapsed_seconds = finite(1);
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
        JobId::parse("uncapped-elapsed").unwrap(),
        "Wait for the local scripted provider".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let running_job = std::sync::Arc::clone(&job);
    let running = tokio::spawn(async move {
        let sink = NoopEventSink;
        running_job.run(&sink).await
    });
    let reached_final_hold = tokio::time::timeout(EVENT_WAIT, async {
        while provider.calls.load(Ordering::Acquire) < 4 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .is_ok();
    // The product value under test is `limits.job.elapsed_seconds = 1` set
    // above; this
    // wait must outlast it to prove uncapped mode does not cancel at it, so it
    // is a product-bound margin and not a load ceiling.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    let remained_active_after_deadline = !running.is_finished();
    provider.release();
    let result = tokio::time::timeout(EVENT_WAIT, running)
        .await
        .expect("the provider release settles the held model call")
        .expect("the native job task does not panic");
    assert!(
        reached_final_hold,
        "the provider reached its held final response"
    );
    assert!(
        remained_active_after_deadline,
        "uncapped mode must not cancel at the configured elapsed ceiling"
    );
    assert!(result.is_ok_and(|output| !output.truncated));
}
