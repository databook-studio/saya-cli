//! A child whose model request exhausts every retry fails on the provider, but
//! a model request cannot change the workspace: the join and the job do not
//! also report uncertain effects, and nothing is left to reconcile. Direct mode
//! does not latch a child's own failure class as the job's failure (the root
//! joined the failed child and answered), so the job carries no failure at all
//! which is the same claim: no uncertain effect is attributed to it. The
//! class the job does not latch is re-pinned where it stays deterministic:
//! the child's own durable completion and the join view of it.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::{
    BudgetDimension, BudgetEffectState, BudgetObservationValue, CodingEventKind,
    CodingFailureClass, CodingResultOutcome, JobId, JobState,
};
use serde_json::Value;
use std::time::Duration;

fn tool_result(provider: &Provider, id: &str) -> Value {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["messages"].as_array().cloned().unwrap_or_default()
        })
        .find(|message| message["tool_call_id"] == id)
        .map(|message| serde_json::from_str(message["content"].as_str().unwrap()).unwrap())
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn an_exhausted_child_request_is_a_provider_failure_not_an_uncertain_effect() {
    let provider = Provider::child_requests_exhausted();
    let mut fixture =
        Fixture::with_endpoint("usage-unknown-exhausted", Some(&provider.endpoint)).await;
    // One retry: the first cut stream is retried, the second exhausts the request.
    fixture.runtime.resolved.ai.retry_delays_ms = vec![5];
    fixture.application = CodingApplication::open(
        fixture.runtime.clone(),
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
        JobId::parse("usage-unknown-exhausted").unwrap(),
        "Write the owned file with helpers".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let run = tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap();
    run.expect("the root answers after joining the failed child");
    assert_eq!(
        job.failure(),
        None,
        "the exhausted child request is not an uncertain effect of the job"
    );
    let terminal = fixture
        .store
        .latest_coding_run_event(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            terminal.event(),
            CodingEventKind::StateChanged {
                state: JobState::Completed
            }
        ),
        "a child's provider failure is not latched as the job's failure: {:?}",
        terminal.event()
    );

    // The class the job does not latch is still recorded where it is
    // deterministic: on the child's own durable completion. Every stream that
    // child received was cut, so its failure is the provider's stream end.
    let completions = fixture
        .store
        .list_coding_completions(job.job_id(), None, 8)
        .await
        .unwrap();
    assert_eq!(completions.items.len(), 1, "{completions:?}");
    let child = &completions.items[0].result;
    assert_eq!(child.agent_id().as_str(), "child-one");
    assert_eq!(child.outcome(), CodingResultOutcome::Failed);
    assert_eq!(
        child.failure_class(),
        Some(CodingFailureClass::ProviderStreamEnded),
        "the exhausted child request is a provider failure"
    );

    // The join is the `helpers` summary: one array element per helper. Every
    // stream the child received was cut, so the summary carries the child's own
    // provider failure class — never a workspace effect the job must reconcile
    // (the recovery status below is the durable form of the same claim).
    let join = tool_result(&provider, "first-join");
    assert_eq!(join[0]["outcome"], "failed", "{join}");
    assert_eq!(
        join[0]["failure_class"], "provider_stream_ended",
        "the join reports the child's own class to the root: {join}"
    );

    let records = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    let request = records
        .items
        .iter()
        .find(|record| {
            record
                .reservation
                .charges()
                .iter()
                .any(|charge| charge.dimension() == BudgetDimension::LogicalRequests)
                && record.reservation.agent_id().is_some()
        })
        .and_then(|record| record.settlement.as_ref())
        .expect("the child's request settled");
    assert!(request.observations().iter().any(|observation| {
        observation.dimension() == BudgetDimension::LogicalRequests
            && observation.value() == BudgetObservationValue::Unknown
    }));
    assert_eq!(request.effect_state(), BudgetEffectState::Settled);
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0
    );
}
