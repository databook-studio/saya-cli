//! The scripted real run and its identity/redaction regressions.

use super::{
    journal,
    provider::{ARG_SENTINEL, Provider, REASONING_SENTINEL, RESULT_SENTINEL},
    support::Fixture,
};
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use std::{sync::Arc, time::Duration};

pub(crate) async fn create(
    label: &str,
    provider: &Provider,
    write_paths: Vec<String>,
) -> (Fixture, Arc<NativeCodingJob>) {
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    create_in(fixture, label, write_paths).await
}

/// The same job, over a fixture whose repository was seeded with `file` before
/// the snapshot was captured, so the run can read content the fixture did not
/// itself create.
pub(crate) async fn create_seeded(
    label: &str,
    provider: &Provider,
    write_paths: Vec<String>,
    file: &str,
    body: String,
) -> (Fixture, Arc<NativeCodingJob>) {
    let fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    std::fs::write(fixture.repository.join(file), body).unwrap();
    create_in(fixture, label, write_paths).await
}

async fn create_in(
    fixture: Fixture,
    label: &str,
    write_paths: Vec<String>,
) -> (Fixture, Arc<NativeCodingJob>) {
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        // The word "helper" is the request's own evidence: the lean tool set
        // advertises `helpers` only when the task asks for one, and this run
        // scripts a real delegation.
        "Produce diagnostic evidence with a helper".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, job)
}
pub(crate) async fn drive(
    label: &str,
    provider: &Provider,
    write_paths: Vec<String>,
) -> (Fixture, Arc<NativeCodingJob>) {
    let (fixture, job) = create(label, provider, write_paths).await;
    tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    (fixture, job)
}

/// `drive` over a repository seeded with `file` before the snapshot.
pub(crate) async fn drive_seeded(
    label: &str,
    provider: &Provider,
    write_paths: Vec<String>,
    file: &str,
    body: String,
) -> (Fixture, Arc<NativeCodingJob>) {
    let (fixture, job) = create_seeded(label, provider, write_paths, file, body).await;
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    (fixture, job)
}

#[tokio::test]
async fn actual_root_child_outcomes_are_private_attributed_and_carry_their_calls() {
    use std::os::unix::fs::PermissionsExt;
    let provider = Provider::diagnostics();
    let (fixture, job) = create("diag-identity", &provider, vec!["diag-a.txt".into()]).await;
    journal::prefill_legacy(&fixture.root, "diag-identity");
    tokio::time::timeout(Duration::from_secs(30), job.run(&NoopEventSink))
        .await
        .unwrap()
        .unwrap();
    assert!(
        journal::records_dir(&fixture.root, "diag-identity")
            .join("tool-events.previous.jsonl")
            .is_file()
    );
    let status = job.status().await.unwrap();
    let bodies = provider.bodies.lock().unwrap().clone();
    let messages: Vec<serde_json::Value> = bodies
        .iter()
        .flat_map(|body| {
            serde_json::from_str::<serde_json::Value>(body).unwrap()["messages"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect();
    assert!(bodies.join("\n").contains(ARG_SENTINEL));
    assert!(
        messages.iter().any(|m| m["role"] == "tool"
            && m["content"]
                .as_str()
                .unwrap_or_default()
                .contains(RESULT_SENTINEL)),
        "the real read must return the file body"
    );
    assert!(
        provider
            .responses
            .lock()
            .unwrap()
            .join("\n")
            .contains(REASONING_SENTINEL)
    );
    assert!(
        messages.iter().any(|m| m["role"] == "tool"
            && m["content"]
                .as_str()
                .unwrap_or_default()
                .contains("\"error\"")),
        "second delegation must be refused as data the model can read"
    );
    assert!(
        journal::records_dir(&fixture.root, "diag-identity")
            .join("request.json")
            .is_file()
    );
    let records = journal::read_journal(&fixture.root, "diag-identity");
    journal::assert_bounded_metadata(&records, "diag-identity");
    // The second `helpers` call reuses the live child's agent id, so the task
    // history refuses it as a tool *error* (raised before any child exists),
    // and the journal records the attempt as `failed`. No delegation refusal is
    // returned as a successful result body any more.
    for (tool, status) in [("helpers", "completed"), ("helpers", "failed")] {
        assert_eq!(journal::count(&records, tool, status), 1, "{records:?}");
    }
    for (tool, status) in [
        ("write", "completed"),
        ("write", "denied"),
        ("read", "completed"),
    ] {
        assert!(
            journal::has_child_record(&records, tool, status, "child-c1"),
            "{records:?}"
        );
    }
    let children = status["children"].as_array().unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["agent_id"], "child-c1");
    // Epoch zero belongs only to the valid preexisting fixture. Every actual
    // root/child event in the new invocation must still carry its true identity.
    for record in records.iter().filter(|record| record["epoch"] != 0) {
        if record["tool"] == "helpers" {
            assert!(record["agent_id"].is_null() && record["invocation_id"].is_null());
        } else {
            assert_eq!(record["agent_id"], "child-c1");
            assert_eq!(record["invocation_id"], children[0]["invocation_id"]);
        }
    }
    let path = journal::journal_path(&fixture.root, "diag-identity");
    assert_eq!(
        std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let text = journal::retained_text(&fixture.root, "diag-identity");
    let exported = fixture
        .application
        .export(job.job_id(), None, None)
        .await
        .unwrap();
    let diagnostic_export = exported["artifacts"]["diagnostics"].to_string();
    assert_eq!(
        exported["artifacts"]["diagnostics"]["history"]["state"],
        "unknown"
    );
    // The journal records each call's own arguments and result, so the task
    // prompt, the file body and the denied path — content the model itself sent
    // or read — are all retained, and a postmortem can tell which call touched
    // which file. Provider reasoning is still never retained: it is not part of
    // any call's arguments or result.
    for marker in [
        ARG_SENTINEL,
        RESULT_SENTINEL,
        "diag-a.txt",
        "diag-denied.txt",
        "unassigned",
    ] {
        assert!(
            text.contains(marker),
            "the retained journal must record the call carrying {marker}"
        );
        assert!(
            diagnostic_export.contains(marker),
            "the diagnostic export must carry {marker}"
        );
    }
    assert!(
        !text.contains(REASONING_SENTINEL),
        "provider reasoning is not a call's arguments or result and is never retained"
    );
    assert!(!diagnostic_export.contains(REASONING_SENTINEL));
}
