//! A direct-mode delegation whose assignment exceeds the `helpers` tool's
//! 32 KiB cap is refused as data the model can correct, before any child,
//! checkout, history or budget effect; assignments at the cap are admitted in
//! the same job and complete.
//!
//! Every assignment the tool admits composes: the child brief bound covers the
//! tool's prompt, description and separator caps, so an assignment at the tool's
//! 32 KiB cap is admitted, composes and completes.
//!
//! The over-cap refusal happens before any effect, so its own tool reservation
//! settles known: the job needs no reconciliation.
use super::{provider::Provider, support::Fixture};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::{BudgetEffectState, JobId, JobState};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, time::Duration};

const REFUSAL: &str = "child task assignment exceeds the assignment byte cap; every child already receives the operator request verbatim, so reference requirement files by path instead of restating them";

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
async fn over_long_assignment_is_refused_before_any_effect_and_a_shortened_one_is_admitted() {
    let provider = Provider::script(false, 44);
    let fixture = Fixture::with_endpoint("delegate-cap", Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse("delegate-cap").unwrap(),
        "Make the two native child changes with helpers".into(),
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let run_job = std::sync::Arc::clone(&job);
    let run = tokio::spawn(async move { run_job.run(&NoopEventSink).await });

    tokio::time::timeout(Duration::from_secs(15), async {
        while !tool_results(&provider).contains_key("over-cap-0") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the parent should observe the result of its first delegation");
    assert_eq!(
        tool_results(&provider)["over-cap-0"],
        json!({"error": REFUSAL}),
        "an over-long assignment is a refusal the model can correct"
    );

    tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .unwrap()
        .unwrap()
        .expect("a refused delegation does not fail the job");
    let results = tool_results(&provider);
    // The `helpers` result is one array element per helper, with the redacted
    // summary of the child it launched.
    let admitted = &results["admitted-0"];
    assert_eq!(admitted[0]["agent"], "child-a", "{admitted}");
    assert_eq!(admitted[0]["outcome"], "completed", "{admitted}");
    let admitted = &results["admitted-1"];
    assert_eq!(admitted[0]["agent"], "child-b", "{admitted}");
    assert_eq!(admitted[0]["outcome"], "completed", "{admitted}");

    let children = fixture
        .store
        .list_coding_children(job.job_id(), None, 10)
        .await
        .unwrap()
        .items;
    let mut agents: Vec<_> = children
        .iter()
        .map(|child| child.agent_id.as_str())
        .collect();
    agents.sort_unstable();
    assert_eq!(
        agents,
        ["child-a", "child-b"],
        "the refused assignment created no child"
    );
    assert!(
        children
            .iter()
            .all(|child| child.state == JobState::Completed)
    );
    // Helpers share the root working checkout W: no checkout is
    // child-owned, so the refused assignment cannot have created one.
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 10)
        .await
        .unwrap()
        .items;
    assert!(
        checkouts
            .iter()
            .all(|record| record.ownership.agent_id().is_none()),
        "the refused assignment created no child checkout"
    );
    let history = fs::read_to_string(fixture.root.join("coding/delegate-cap/tasks.jsonl")).unwrap();
    assert_eq!(
        history.lines().count(),
        2,
        "only the two admitted batches are durable"
    );
    assert!(!history.contains("child-over"));

    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    assert!(reservations.next_cursor.is_none());
    assert!(
        reservations
            .items
            .iter()
            .all(|record| record.settlement.is_some()),
        "every reservation settled"
    );
    // The refused call created no delegation effect, and its own tool
    // reservation is settled known: the refusal (`AssignmentTooLong`) is raised
    // before any child is registered, so it proves no effect.
    let uncertain = reservations
        .items
        .iter()
        .filter(|record| {
            record
                .settlement
                .as_ref()
                .is_some_and(|settlement| settlement.effect_state() == BudgetEffectState::Uncertain)
        })
        .count();
    assert_eq!(
        uncertain, 0,
        "the over-long call never ran, so nothing is left uncertain"
    );
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0,
        "a refusal before any effect leaves the job with nothing to reconcile"
    );
    let record = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.state,
        JobState::Completed,
        "a refused delegation does not fail the job; a clean run ends completed"
    );
}
