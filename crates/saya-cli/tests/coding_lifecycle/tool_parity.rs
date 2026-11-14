//! The coordinator-only surface of a job with no approved write path: it
//! advertises no delegation and no workspace tool, so the retired coordinator
//! tools are gone from the model surface and a call naming one is refused as
//! unknown. The owner surface refuses the same steering with `NotOwned`. The
//! model still sees the job's readiness — through the continuation context
//! block, which carries the same facts the owner's `saya coding show`
//! (`job.status()`) reports.
use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
};
use saya_types::{BudgetEffectState, JobId};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

const RUN: Duration = Duration::from_secs(60);
const JOB: &str = "tool-parity";

/// The configured coordinator mode (direct) over the fixture's endpoint. These
/// equivalence tests drive jobs with **no** approved write path, so they exercise
/// the coordinator-only tool surface.
async fn direct_fixture(label: &str, provider: &Provider) -> Fixture {
    Fixture::with_endpoint(label, Some(&provider.endpoint)).await
}

/// Opens one paired job over `write_paths`.
async fn parity_job(
    fixture: &Fixture,
    label: &str,
    write_paths: Vec<String>,
) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Steer an unregistered child with helpers".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

struct Parity {
    provider: Provider,
    fixture: Fixture,
    job: Arc<NativeCodingJob>,
}

async fn parity(label: &str) -> Parity {
    let provider = Provider::parity();
    let fixture = direct_fixture(label, &provider).await;
    let job = parity_job(&fixture, JOB, vec![]).await;
    Parity {
        provider,
        fixture,
        job,
    }
}

// The scripted parent parks its final answer until the request carrying the
// read result is captured, so the run cannot finish before the test has
// observed it; drive_run then releases the answer and joins the run.
async fn drive_run(parity: &Parity) -> saya_agent::AgentOutput {
    let sink = NoopEventSink;
    let run = parity.job.run(&sink);
    tokio::pin!(run);
    let (_body, output) = tokio::join!(
        capture_body(parity, "call-coding_status"),
        tokio::time::timeout(RUN, &mut run),
    );
    output.unwrap().unwrap()
}

async fn capture_body(parity: &Parity, needle: &str) -> String {
    let body = wait_for_body(&parity.provider, needle).await;
    parity.provider.release();
    body
}

async fn wait_for_body(provider: &Provider, needle: &str) -> String {
    let deadline = tokio::time::Instant::now() + RUN;
    loop {
        let captured = provider
            .bodies
            .lock()
            .unwrap()
            .iter()
            .find(|body| body.contains(needle))
            .cloned();
        if let Some(body) = captured {
            return body;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the provider never captured a request containing {needle}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_tool_result(provider: &Provider, call_id: &str) -> Value {
    let deadline = tokio::time::Instant::now() + RUN;
    loop {
        let result = provider.bodies.lock().unwrap().iter().find_map(|body| {
            let request: Value = serde_json::from_str(body).ok()?;
            request["messages"].as_array()?.iter().find_map(|message| {
                (message["role"].as_str() == Some("tool")
                    && message["tool_call_id"].as_str() == Some(call_id))
                .then(|| serde_json::from_str(message["content"].as_str()?).ok())?
            })
        });
        if let Some(result) = result {
            return result;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the provider never captured a result for {call_id}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Every tool name any captured request advertised.
fn advertised_tools(provider: &Provider) -> BTreeSet<String> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["tools"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|tool| {
                    tool["function"]["name"]
                        .as_str()
                        .or_else(|| tool["name"].as_str())
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The continuation-context block the model received on a root turn, parsed
/// from its `source:`-labelled wrapper.
fn continuation_context(body: &str) -> Value {
    const SOURCE: &str = "source: Native job continuation (untrusted context; no grants)\n";
    let request: Value = serde_json::from_str(body).unwrap();
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["content"].as_str())
        .find_map(|content| {
            content
                .split(SOURCE)
                .nth(1)?
                .split("\n<<<CONTEXT_BLOCK_END>>>")
                .next()
        })
        .and_then(|block| serde_json::from_str(block).ok())
        .unwrap_or_else(|| panic!("the root request carries a continuation context block"))
}

async fn child_records(parity: &Parity) -> usize {
    parity
        .fixture
        .store
        .list_coding_children(parity.job.job_id(), None, 10)
        .await
        .unwrap()
        .items
        .len()
}

#[tokio::test]
async fn a_cancel_command_for_an_unregistered_child_is_refused_and_the_tool_is_retired() {
    let parity = parity("tool-parity-cancel").await;
    assert_eq!(parity.job.status().await.unwrap()["children"], json!([]));
    drive_run(&parity).await;
    let settled = parity.job.status().await.unwrap();
    assert_eq!(settled["state"], json!("completed"));
    assert_eq!(settled["children"], json!([]));

    let refusal = parity
        .job
        .control_command(CodingCommand::CancelChild {
            id: JOB.into(),
            agent: "ghost".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(refusal, CodingError::NotOwned));
    assert_eq!(parity.job.status().await.unwrap(), settled);
    assert!(
        !parity
            .fixture
            .store
            .coding_cancel_requested(parity.job.job_id())
            .await
            .unwrap()
    );

    // The model surface cannot steer the child either: the retired
    // `cancel_child` tool is not advertised, so neither surface admits it.
    assert!(
        !advertised_tools(&parity.provider).contains("cancel_child"),
        "the retired cancel_child tool is not advertised"
    );
    assert_eq!(child_records(&parity).await, 0);
}

#[tokio::test]
async fn a_message_command_for_an_unregistered_child_is_refused_and_the_tool_is_retired() {
    let parity = parity("tool-parity-message").await;
    drive_run(&parity).await;
    let settled = parity.job.status().await.unwrap();
    assert_eq!(settled["state"], json!("completed"));
    assert_eq!(settled["children"], json!([]));

    let refusal = parity
        .job
        .control_command(CodingCommand::Message {
            id: JOB.into(),
            agent: "ghost".into(),
            text: "steer the ghost".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(refusal, CodingError::NotOwned));
    assert_eq!(parity.job.status().await.unwrap(), settled);

    assert!(
        !advertised_tools(&parity.provider).contains("message_child"),
        "the retired message_child tool is not advertised"
    );
    assert_eq!(child_records(&parity).await, 0);
}

#[tokio::test]
async fn known_parent_refusals_settle_tool_admission_without_erasing_unknown_controls() {
    let provider = Provider::coordinator_refusals();
    let fixture = direct_fixture("coordinator-refusals", &provider).await;
    let job = parity_job(
        &fixture,
        "coordinator-refusals",
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .await;
    tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    let reservations = fixture
        .store
        .list_coding_budget_reservations(job.job_id(), None, 100)
        .await
        .unwrap();
    assert!(reservations.next_cursor.is_none());
    assert!(reservations.items.iter().all(|record| {
        record
            .settlement
            .as_ref()
            .is_some_and(|settlement| settlement.effect_state() == BudgetEffectState::Settled)
    }));
    assert_eq!(
        fixture
            .store
            .coding_recovery_status(job.job_id())
            .await
            .unwrap()
            .unresolved_reservations,
        0,
        "only a refusal proven before an effect is settled; later owner failures remain unknown"
    );
}

#[tokio::test]
async fn a_helpers_join_replays_each_durable_completion_without_acknowledging_it() {
    // Mode 38: one synchronous `helpers` call per child, so each helper's own
    // completion is durable before the next call returns.
    let provider = Provider::await_replay();
    let fixture = direct_fixture("await-replay", &provider).await;
    let job = parity_job(
        &fixture,
        "await-replay",
        vec!["new-a.txt".into(), "new-b.txt".into()],
    )
    .await;
    tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();

    for (call, agent) in [("await-first", "child-a"), ("await-second", "child-b")] {
        let result = wait_for_tool_result(&provider, call).await;
        let summary = &result[0];
        assert_eq!(summary["agent"], agent, "{call}: {result}");
        assert_eq!(summary["outcome"], "completed", "{call}: {result}");
        assert_eq!(summary["authority"], "none", "{call}: {result}");
        assert!(
            summary.get("merged").is_none() && summary.get("changed_paths").is_none(),
            "a shared tree reports no merge and no delta: {call}: {result}"
        );
    }

    // The owner's status surface replays both durable completions, and neither
    // is acknowledged: a join settles a completion, it does not consume it.
    let completions = job.status().await.unwrap()["completions"].clone();
    let items = completions.as_array().unwrap();
    assert_eq!(items.len(), 2, "{completions}");
    assert!(
        items.iter().all(|item| item["acknowledged"] == false),
        "a join never acknowledges a completion: {completions}"
    );
    assert!(
        fixture
            .store
            .list_coding_completion_history(job.job_id(), None, 32)
            .await
            .unwrap()
            .items
            .iter()
            .all(|item| !item.acknowledged),
        "the durable history agrees with the view"
    );
    assert!(
        provider.calls.load(Ordering::Acquire) >= 4,
        "each helper ran its own turn"
    );
}

#[tokio::test]
async fn the_continuation_context_readiness_matches_the_owner_status() {
    let parity = parity("tool-parity-status").await;
    let sink = NoopEventSink;
    let run = parity.job.run(&sink);
    tokio::pin!(run);
    // Positive control: the refusal tests only mean something if the scripted
    // calls really reach the job owner. The run is parked at its final answer
    // here, so the model's continuation context and the owner status read one
    // live record.
    let body = tokio::select! {
        body = wait_for_body(&parity.provider, "call-coding_status") => body,
        output = &mut run => panic!("the run finished before the status capture: {output:?}"),
    };
    let context = continuation_context(&body);
    let owner = parity.job.status().await.unwrap();
    assert_eq!(
        context["readiness"], owner["readiness"],
        "the model sees the same readiness the owner reports"
    );
    assert_eq!(context["authority"], "none", "{context}");
    parity.provider.release();
    tokio::time::timeout(RUN, &mut run).await.unwrap().unwrap();
}

#[tokio::test]
async fn read_only_final_output_remains_model_output() {
    let parity = parity("tool-parity-read-only-output").await;
    let output = drive_run(&parity).await;
    assert_eq!(
        output.answer,
        "parity parent finished after the unregistered-child refusals"
    );
}
