//! Durable prepared-candidate state: the receipt is immutable per source
//! generation, a replay returns it exactly, and an incomplete or unreadable
//! candidate state is refused before another candidate effect. The retired
//! `coding_prepare_candidate` tool has no replacement, so every preparation
//! here is the OWNER's at the end of a run.

use super::direct_prepare::fixture::{checkout_path, resume};
use super::{provider::Provider, support::Fixture};
use saya_agent::{AgentOutput, CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use serde_json::Value;
use std::{collections::BTreeMap, fs, sync::Arc, time::Duration};

/// Runs a job to completion and returns its own result.
pub(super) async fn run(job: &Arc<saya_cli::NativeCodingJob>) -> AgentOutput {
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap()
}

/// The final result a write-path run ends with: the owner's readiness envelope.
pub(super) fn final_envelope(output: &AgentOutput) -> Value {
    serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope")
}

/// Resumes the job under a fresh scoped approval and runs it.
pub(super) async fn run_resumed(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
) -> (Arc<saya_cli::NativeCodingJob>, AgentOutput) {
    let resumed = resume(fixture, job).await;
    let output = run(&resumed).await;
    (resumed, output)
}

#[path = "prepare_candidate/early_prepare.rs"]
mod early_prepare;
#[path = "prepare_candidate/malformed_receipt.rs"]
mod malformed_receipt;
#[path = "prepare_candidate/partial_candidate.rs"]
mod partial_candidate;
#[path = "prepare_candidate/replay_resume.rs"]
mod replay_resume;
/// Opens a direct job over `write_paths` without running it. The same helper
/// serves the convergence group and this group's own tests.
pub(super) async fn job_with_paths(
    fixture: &Fixture,
    label: &str,
    write_paths: &[&str],
) -> std::sync::Arc<saya_cli::NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Prepare but never apply one candidate with helpers".into(),
        write_paths.iter().map(|path| (*path).to_owned()).collect(),
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

/// One candidate over the tracked file.
pub(super) async fn job(
    fixture: &Fixture,
    label: &str,
) -> std::sync::Arc<saya_cli::NativeCodingJob> {
    job_with_paths(fixture, label, &["tracked.txt"]).await
}

/// The group's fixture: a direct-mode job over the fixture repository. Checks are removed, so no check profile is configured: a
/// prepared candidate is ready by construction.
pub(super) async fn direct_fixture(label: &str, provider: &Provider) -> Fixture {
    let mut fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    // Preparation scenarios script repeated reads to exercise the owner's
    // preparation, not the job's progress policy.
    fixture.runtime.resolved.limits.job.no_progress_turns = 0;
    fixture.runtime.resolved.limits.job.repeat_limit = 0;
    fixture.program_dir = super::support::program_dir_for(&["grep"]);
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
    fixture
}

#[tokio::test]
async fn a_fresh_write_job_is_not_candidate_ready_before_preparation() {
    // The job is never run: only its status is read, so any script will do.
    let provider = Provider::new(false);
    let fixture = direct_fixture("prepare-fresh-direct", &provider).await;
    let job = job_with_paths(&fixture, "prepare-fresh-direct", &["tracked.txt"]).await;

    // Nothing has run yet: no root working checkout W, no generation, no
    // receipt. The advisory view reports the facts and forces no tool call.
    let status = job.status().await.unwrap();
    let readiness = &status["readiness"];
    assert_eq!(readiness["state"], "unknown", "{status}");
    assert_eq!(readiness["candidate_ready"], false, "{status}");
    assert_eq!(readiness["w_changed"], false, "{status}");
    assert_eq!(readiness["suggestion"], "none", "{status}");
    assert_eq!(
        readiness["live_children"],
        serde_json::json!([]),
        "{status}"
    );
    assert!(status["next_action"].is_null(), "{status}");
}

#[tokio::test]
async fn a_child_that_changes_nothing_still_settles_a_completion() {
    // Mode 87: the root writes `new-a.txt` in W, hands `new-b.txt` to a helper
    // that changes nothing, and answers. The helper settles its own completion
    // and leaves no delta, so the prepared candidate is the root's own change
    // and the helper's summary says so.
    let provider = Provider::direct_prepare_empty_child();
    let fixture = direct_fixture("prepare-empty", &provider).await;
    let job = job_with_paths(&fixture, "prepare-empty", &["new-a.txt", "new-b.txt"]).await;
    let output = run(&job).await;
    let results = tool_results(&provider);
    let summary = &results["empty-task"][0];
    assert!(
        results["empty-task"].get("error").is_none(),
        "task: {}",
        results["empty-task"]
    );
    assert_eq!(summary["agent"], "child-b", "{summary}");
    assert_eq!(summary["outcome"], "completed", "{summary}");
    assert!(
        summary.get("changed_paths").is_none() && summary.get("merged").is_none(),
        "a helper writes W directly, so the summary reports no delta and no merge: {summary}"
    );
    let completions = fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items;
    assert_eq!(completions.len(), 1, "{completions:?}");
    assert_eq!(completions[0].result.agent_id().as_str(), "child-b");
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    let checkout = checkout_path(&fixture, &job, envelope["candidate"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read_to_string(checkout.join("new-a.txt")).unwrap(),
        "the root's own delta\n",
        "the candidate carries the root's change"
    );
    assert!(
        !checkout.join("new-b.txt").exists(),
        "the helper that changed nothing left no path in the candidate"
    );
}

/// The durable prepared receipt one source generation owns.
fn receipt_path(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
    digest: &str,
) -> std::path::PathBuf {
    fixture
        .root
        .join("coding")
        .join(job.job_id().as_str())
        .join(format!("prepared-candidate-{digest}.json"))
}

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
