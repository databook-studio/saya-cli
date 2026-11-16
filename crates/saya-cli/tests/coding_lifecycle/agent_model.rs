//! A helper type may carry a model of its own.
//!
//! `[agents.<name>] model` decides which model that type's helpers ask
//! the job's own endpoint for. The endpoint — provider, base URL and
//! credential — is still the job's: a model name selects what answers, never
//! where the request goes. With no `model`, the helper sends exactly the model
//! the job itself sends, as it always did.
//!
//! Both runs use mode 62: the root delegates one piece to a `general` helper,
//! then writes a second file itself, so one run's captured bodies hold both a
//! helper's request and the root's. Every body here was captured by the
//! fixture's own transport, so a helper's request provably went to the job's
//! endpoint whatever model it names: a `model` cannot point one elsewhere.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::time::Duration;

/// The model one captured request body asked for.
fn model_of(body: &str) -> String {
    let body: Value = serde_json::from_str(body).expect("a captured body is JSON");
    body["model"]
        .as_str()
        .expect("every provider request names a model")
        .to_owned()
}

/// The bodies a run sent, split into the root's own requests and its helpers'.
/// The child brief is what tells them apart, exactly as the transport does.
fn bodies(provider: &Provider) -> (Vec<String>, Vec<String>) {
    provider
        .bodies
        .lock()
        .unwrap()
        .clone()
        .into_iter()
        .partition(|body| !body.contains("UNTRUSTED CHILD TASK"))
}

/// Runs mode 62 under the given `[agents]` table.
async fn run(label: &str, agents: &str) -> (Fixture, Provider) {
    let provider = Provider::script(false, 62);
    let fixture = Fixture::with_agents(label, &provider.endpoint, agents).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Delegate one file and write the other".into(),
        vec!["file-a.txt".into(), "file-b.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    (fixture, provider)
}

/// A type's `model` reaches that type's helper: every request the helper sends
/// asks for it, while the root's own requests keep the job's model.
#[tokio::test]
async fn a_configured_agent_model_reaches_its_helper_request() {
    let (_fixture, provider) = run(
        "agent-model",
        "[agents.general]\nmodel = \"helper-model\"\n",
    )
    .await;
    let (root, helpers) = bodies(&provider);
    assert!(
        !root.is_empty() && !helpers.is_empty(),
        "the run must have sent both a root request and a helper request"
    );
    for body in &helpers {
        assert_eq!(
            model_of(body),
            "helper-model",
            "the type's own model answers its helpers"
        );
    }
    for body in &root {
        assert_eq!(
            model_of(body),
            "native-fixture",
            "the root keeps the job's model"
        );
    }
}

/// With no `model`, the helper's request is the one it always was: the job's
/// own model, byte for byte, from the same endpoint.
#[tokio::test]
async fn an_agent_without_a_model_sends_the_jobs_own_model_unchanged() {
    let (_fixture, provider) = run(
        "agent-model-unset",
        "[agents.general]\nallow_write = true\n",
    )
    .await;
    let (root, helpers) = bodies(&provider);
    assert!(
        !root.is_empty() && !helpers.is_empty(),
        "the run must have sent both a root request and a helper request"
    );
    for body in &helpers {
        assert_eq!(
            model_of(body),
            model_of(&root[0]),
            "an unset model leaves the helper on the job's own model, byte for byte"
        );
    }
    assert_eq!(model_of(&root[0]), "native-fixture");
}
