use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
};
use saya_types::JobId;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(20);
const JOB: &str = "tool-parity";

struct Parity {
    provider: Provider,
    fixture: Fixture,
    job: Arc<NativeCodingJob>,
}

async fn parity(label: &str) -> Parity {
    let provider = Provider::parity();
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let request = CodingJobRequest::new(
        JobId::parse(JOB).unwrap(),
        "Steer an unregistered child".into(),
        vec![],
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    Parity {
        provider,
        fixture,
        job,
    }
}

// The scripted parent parks its final answer until the request carrying the
// coding_status result is captured, so the run cannot finish before the test
// has observed it; drive_run then releases the answer and joins the run.
async fn drive_run(parity: &Parity) {
    let sink = NoopEventSink;
    let run = parity.job.run(&sink);
    tokio::pin!(run);
    let (_status, output) = tokio::join!(
        capture_status_result(parity),
        tokio::time::timeout(RUN, &mut run),
    );
    output.unwrap().unwrap();
}

async fn capture_status_result(parity: &Parity) -> Value {
    let body = wait_for_body(&parity.provider, "call-coding_status").await;
    let content = tool_content(&body, "call-coding_status");
    parity.provider.release();
    content
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

fn tool_content(body: &str, call_id: &str) -> Value {
    let request: Value = serde_json::from_str(body).unwrap();
    let message = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| {
            message["role"].as_str() == Some("tool")
                && message["tool_call_id"].as_str() == Some(call_id)
        })
        .unwrap_or_else(|| panic!("no tool result for {call_id} in the captured request"));
    serde_json::from_str(message["content"].as_str().unwrap()).unwrap()
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
async fn cancel_child_for_an_unregistered_agent_is_refused_identically_on_both_surfaces() {
    let parity = parity("tool-parity-cancel").await;
    assert_eq!(parity.job.status().await.unwrap()["children"], json!([]));
    drive_run(&parity).await;
    let settled = parity.job.status().await.unwrap();
    assert_eq!(settled["state"], json!("completed"));
    assert_eq!(settled["children"], json!([]));
    assert!(
        !parity
            .fixture
            .store
            .coding_cancel_requested(parity.job.job_id())
            .await
            .unwrap()
    );

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

    let body = wait_for_body(&parity.provider, "call-cancel_child").await;
    assert_eq!(
        tool_content(&body, "call-cancel_child"),
        json!({"error": refusal.to_string()}),
    );
    assert_eq!(child_records(&parity).await, 0);
}

#[tokio::test]
async fn message_child_for_an_unregistered_agent_is_refused_identically_on_both_surfaces() {
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
        !parity
            .fixture
            .store
            .coding_cancel_requested(parity.job.job_id())
            .await
            .unwrap()
    );

    let body = wait_for_body(&parity.provider, "call-message_child").await;
    assert_eq!(
        tool_content(&body, "call-message_child"),
        json!({"error": refusal.to_string()}),
    );
    assert_eq!(child_records(&parity).await, 0);
}

#[tokio::test]
async fn model_issued_coding_status_matches_the_owner_status() {
    let parity = parity("tool-parity-status").await;
    let sink = NoopEventSink;
    let run = parity.job.run(&sink);
    tokio::pin!(run);
    // Positive control: the refusal tests only mean something if the scripted
    // tool calls really reach the job owner. The run is parked at its final
    // answer here, so the owner status and the tool status read one live
    // record.
    let body = tokio::select! {
        body = wait_for_body(&parity.provider, "call-coding_status") => body,
        output = &mut run => panic!("the run finished before the status capture: {output:?}"),
    };
    let tool_status = tool_content(&body, "call-coding_status");
    let owner = parity.job.status().await.unwrap();
    assert_eq!(tool_status["job_id"], owner["job_id"]);
    assert_eq!(tool_status["state"], owner["state"]);
    assert_eq!(tool_status["children"], owner["children"]);
    parity.provider.release();
    tokio::time::timeout(RUN, &mut run).await.unwrap().unwrap();
}
