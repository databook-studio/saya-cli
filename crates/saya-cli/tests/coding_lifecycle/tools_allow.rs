//! The one `[tools] allow` list, end to end on the wire.
//!
//! A job that states a list advertises exactly that list, and its helpers get
//! the job's set minus `helpers` — asserted on the request bodies the run
//! actually sent, not on the composition the test believes it built. The same
//! script runs unnarrowed beside it, so the narrowed run is a difference in
//! the advertised names rather than a difference in the script.

use super::direct_task::root_checkout;
use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

/// The tool names one captured request body advertised, sorted.
fn advertised(body: &str) -> Vec<String> {
    let body: Value = serde_json::from_str(body).expect("a captured body is JSON");
    let mut names: Vec<String> = body["tools"]
        .as_array()
        .expect("every coding request advertises its tools")
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .expect("every advertised tool is named")
                .to_string()
        })
        .collect();
    names.sort();
    names
}

/// The bodies the run sent, in order.
fn bodies(provider: &Provider) -> Vec<String> {
    provider.bodies.lock().unwrap().clone()
}

/// The root's first request: the one that carries no child assignment.
fn root_body(provider: &Provider) -> String {
    bodies(provider)
        .into_iter()
        .find(|body| !body.contains("UNTRUSTED CHILD TASK"))
        .expect("the root requested before any helper did")
}

/// The helper's first request.
fn helper_body(provider: &Provider) -> String {
    bodies(provider)
        .into_iter()
        .find(|body| body.contains("UNTRUSTED CHILD TASK"))
        .expect("the delegated helper requested")
}

/// Runs mode 62 — the root delegates `file-a.txt` and writes `file-b.txt` —
/// under the given stated list, and returns the finished job and its provider.
async fn run(
    label: &str,
    stated: Option<Vec<String>>,
) -> (Fixture, Provider, Arc<NativeCodingJob>) {
    let provider = Provider::script(false, 62);
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    let mut request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Delegate one file and write the other".into(),
        vec!["file-a.txt".into(), "file-b.txt".into()],
    )
    .unwrap();
    if stated.is_some() {
        request = request.with_tools_allow(stated).unwrap();
    }
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
    (fixture, provider, job)
}

#[tokio::test]
async fn a_stated_list_narrows_the_root_and_the_helpers_on_the_wire() {
    let (fixture, provider, job) = run(
        "tools-allow-narrowed",
        Some(vec!["helpers".into(), "write".into()]),
    )
    .await;

    // The root advertises the stated list and nothing else: not the workspace
    // tools it would otherwise compose, and not `checklist`.
    assert_eq!(
        advertised(&root_body(&provider)),
        vec!["helpers".to_string(), "write".to_string()],
        "the root advertises exactly the stated list"
    );
    // The helper gets the job's set minus `helpers`: only the root delegates,
    // so a helper can never be handed the tool it would need to delegate again.
    assert_eq!(
        advertised(&helper_body(&provider)),
        vec!["write".to_string()],
        "a helper's set is the job's minus `helpers`"
    );

    // The narrowed set was sufficient: the scripted run completed its work.
    let working = root_checkout(&fixture, &job).await;
    assert_eq!(
        std::fs::read_to_string(working.join("file-a.txt")).unwrap(),
        "child-a wrote\n"
    );
    assert_eq!(
        std::fs::read_to_string(working.join("file-b.txt")).unwrap(),
        "root wrote after the merge\n"
    );
}

#[tokio::test]
async fn an_unstated_list_advertises_what_the_job_always_did() {
    let (_fixture, provider, _job) = run("tools-allow-default", None).await;
    let root = advertised(&root_body(&provider));
    for name in ["helpers", "checklist", "read", "write"] {
        assert!(
            root.contains(&name.to_string()),
            "an unstated list narrows nothing; {name} is missing from {root:?}"
        );
    }
    let helper = advertised(&helper_body(&provider));
    assert!(
        helper.contains(&"checklist".to_string()) && helper.contains(&"write".to_string()),
        "a helper composes the child surface as it always did: {helper:?}"
    );
    assert!(
        !helper.contains(&"helpers".to_string()),
        "only the root delegates: {helper:?}"
    );
}
