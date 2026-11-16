//! A child whose first model stream is cut recovers on retry. Only the usage of
//! the cut attempt is unknown: the join, the job and the operator follow-up of
//! that child are not blocked as if a workspace effect were uncertain. Direct
//! mode advertises no `follow_up_child` root tool, so the follow-up runs
//! through the surviving operator surface (`coding follow-up`) on the resumed
//! job, over the same child checkout.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{ChildFollowUpRequest, CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::{BudgetDimension, BudgetEffectState, BudgetObservationValue, JobId};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

fn tool_results(provider: &Provider) -> BTreeMap<String, Value> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|message| {
                    let id = message["tool_call_id"].as_str()?;
                    Some((
                        id.to_owned(),
                        serde_json::from_str(message["content"].as_str()?).unwrap(),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[tokio::test]
async fn a_cut_child_stream_neither_fails_the_job_nor_blocks_the_follow_up() {
    let provider = Provider::child_stream_cut_once();
    let mut fixture = Fixture::with_endpoint("usage-unknown-cut", Some(&provider.endpoint)).await;
    // The fixture allows no retry (an empty schedule means none); the cut
    // stream needs exactly one.
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
        JobId::parse("usage-unknown-cut").unwrap(),
        "Write the owned file, then follow the child up with helpers".into(),
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

    let results = tool_results(&provider);
    // The join is the `helpers` summary: one array element per helper. The
    // child recovered on the retry of its cut stream, so its own outcome is
    // `completed` and it carries no failure class — the join of a child that
    // only lost one stream is not an uncertain effect.
    let summary = &results["first-join"][0];
    assert_eq!(
        summary["outcome"], "completed",
        "the child recovered on the retry of its cut stream: {summary}"
    );
    assert!(
        summary["failure_class"].is_null(),
        "a recovered child records no failure class: {summary}"
    );
    run.expect("the job does not fail for uncertainty");

    // The operator follow-up of the same child is not blocked as if the cut
    // attempt had left a workspace effect uncertain.
    let completion = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .pop()
        .unwrap();
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap();
    let follow_up = resumed
        .run_follow_up(ChildFollowUpRequest {
            agent_id: completion.result.agent_id().clone(),
            expected_completion_sequence: completion.sequence,
            instruction: "write for child-one; second pass".into(),
        })
        .await;
    assert!(
        follow_up.is_ok(),
        "follow-up after a cut attempt: {follow_up:?}"
    );
    assert_eq!(
        fixture
            .store
            .list_coding_completion_history(job.job_id(), None, 32)
            .await
            .unwrap()
            .items
            .len(),
        2,
        "the follow-up child completed its own invocation"
    );

    let records = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    let unknown_attempts = records
        .items
        .iter()
        .filter_map(|record| record.settlement.as_ref())
        .filter(|settlement| {
            settlement.observations().iter().any(|observation| {
                observation.dimension() == BudgetDimension::TransportAttempts
                    && observation.value() == BudgetObservationValue::Unknown
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(unknown_attempts.len(), 1, "the cut attempt stays visible");
    assert_eq!(
        unknown_attempts[0].effect_state(),
        BudgetEffectState::Settled
    );
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
