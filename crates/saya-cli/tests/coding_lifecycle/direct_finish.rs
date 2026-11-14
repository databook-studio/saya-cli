//! Direct-mode finish: when the root loop ends, the OWNER prepares the
//! candidate once if the working checkout W changed, no child is live and no
//! receipt exists for the current generation — so readiness always exists
//! without the model being told to prepare. A preparation failure is reported
//! as not ready (the existing unprepared-candidate outcome), never as a root
//! failure, and it is NAMED by a closed reason.

use super::direct_prepare::fixture::{
    checkout_path, checkouts_named, prepared_receipts, records_dir, resume,
    run_direct_prepare_output,
};
use super::direct_task::{root_checkout, tool_result};
use super::support::Fixture;
use saya_agent::NoopEventSink;
use saya_cli::NativeCodingJob;
use saya_types::JobState;
use serde_json::Value;
use std::time::Duration;

/// The final result a write-path run ends with: the owner's readiness envelope.
fn final_envelope(output: &saya_agent::AgentOutput) -> Value {
    serde_json::from_str(&output.answer).expect("the run's answer is the readiness envelope")
}

/// The closed refusal the not-ready view names, or a panic naming the view.
fn preparation(envelope: &Value) -> &Value {
    envelope["readiness"]
        .get("preparation")
        .unwrap_or_else(|| panic!("the not-ready view names the preparation reason: {envelope}"))
}

async fn job_state(fixture: &Fixture, job: &NativeCodingJob) -> JobState {
    fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .expect("the job record exists")
        .state
}

/// Runs a resumed job to completion and returns its own result.
async fn run_resumed(job: &std::sync::Arc<NativeCodingJob>) -> saya_agent::AgentOutput {
    tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the resumed run terminates")
        .unwrap()
}

#[tokio::test]
async fn a_direct_job_that_never_prepares_ends_with_an_owner_prepared_candidate() {
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-finish-owner",
        75,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    let digest = envelope["source_generation_sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("the owner prepared a candidate: {envelope}"));
    assert_eq!(
        envelope["candidate"].as_str().unwrap(),
        format!("candidate-{digest}")
    );
    assert_eq!(
        envelope["apply_requires_fresh_approval"], true,
        "{envelope}"
    );
    assert_eq!(envelope["w_changed"], true, "{envelope}");
    assert_eq!(envelope["suggestion"], "none", "{envelope}");
    assert!(
        envelope.get("verification_scope").is_none(),
        "the direct envelope carries the direct facts, not the child-sourced ones: {envelope}"
    );
    assert!(
        records_dir(&fixture, &job)
            .join(format!("prepared-candidate-{digest}.json"))
            .exists(),
        "the owner's receipt is durable for the current generation"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "exactly the owner-prepared candidate exists"
    );
    let candidate = checkout_path(&fixture, &job, envelope["candidate"].as_str().unwrap()).await;
    assert_eq!(
        std::fs::read_to_string(candidate.join("added.txt")).unwrap(),
        "FIXED\n",
        "the candidate carries W's delta"
    );
    assert!(job.failure().is_none(), "preparation is not a root failure");
    assert_eq!(job_state(&fixture, &job).await, JobState::Completed);
}

#[tokio::test]
async fn a_direct_job_with_no_change_prepares_nothing() {
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-finish-unchanged",
        76,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], false, "{envelope}");
    assert_eq!(envelope["w_changed"], false, "{envelope}");
    assert_eq!(envelope["suggestion"], "none", "{envelope}");
    assert_eq!(
        prepared_receipts(&fixture, &job),
        0,
        "an unchanged working checkout prepares no candidate"
    );
    assert_eq!(checkouts_named(&fixture, &job, "candidate-").await, 0);
    assert!(job.failure().is_none());
    assert!(
        root_checkout(&fixture, &job).await.exists(),
        "the working checkout W itself still exists"
    );
}

#[tokio::test]
async fn an_owner_preparation_failure_is_reported_as_not_ready_not_a_root_failure() {
    // The first run's owner prepared a candidate; the operator removes its
    // receipt, so the resumed run's owner tries to prepare again and finds the
    // generation's candidate checkout without one: an incomplete candidate
    // state. The refusal is swallowed as the existing not-ready outcome.
    let (fixture, job, _provider, output) = run_direct_prepare_output(
        "direct-finish-notready",
        75,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let first = final_envelope(&output);
    assert_eq!(first["candidate_ready"], true, "{first}");
    let digest = first["source_generation_sha256"].as_str().unwrap();
    std::fs::remove_file(
        records_dir(&fixture, &job).join(format!("prepared-candidate-{digest}.json")),
    )
    .unwrap();

    let resumed = resume(&fixture, &job).await;
    let envelope = final_envelope(&run_resumed(&resumed).await);
    assert_eq!(envelope["candidate_ready"], false, "{envelope}");
    assert_eq!(envelope["w_changed"], true, "{envelope}");
    let preparation = preparation(&envelope);
    assert_eq!(preparation["reason"], "candidate_state", "{envelope}");
    assert!(
        preparation["detail"]
            .as_str()
            .is_some_and(|text| !text.is_empty()),
        "the reason carries one short fixed sentence: {envelope}"
    );
    assert!(
        preparation
            .as_object()
            .is_some_and(|fields| fields.len() == 2),
        "the reason is closed: only a name and one sentence: {envelope}"
    );
    assert_eq!(
        prepared_receipts(&fixture, &job),
        0,
        "the refused preparation wrote no receipt"
    );
    assert_eq!(checkouts_named(&fixture, &job, "candidate-").await, 1);
    assert!(job.failure().is_none(), "preparation is not a root failure");
    assert_eq!(
        job_state(&fixture, &job).await,
        JobState::Completed,
        "the job ends like any other not-ready candidate"
    );
}

#[tokio::test]
async fn a_direct_root_with_no_children_is_never_told_to_finish() {
    // Mode 78: the root writes W, then reads the checkout where an
    // `await_children` call would be — with no children there is nothing to await
    // and nothing to be told. The owner still prepares the candidate.
    let (fixture, job, provider, output) = run_direct_prepare_output(
        "direct-finish-await",
        78,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let awaited = tool_result(&provider, "await-advisory");
    assert_eq!(
        awaited["path"].as_str(),
        Some("tracked.txt"),
        "the turn is a plain read of the checkout, not a lifecycle instruction: {awaited}"
    );
    assert!(
        awaited.get("next_action").is_none(),
        "a direct view carries no forced action: {awaited}"
    );
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    assert_eq!(envelope["w_changed"], true, "{envelope}");
    assert_eq!(envelope["suggestion"], "none", "{envelope}");
    assert!(
        envelope["next_action"].is_null(),
        "the direct readiness forces no finish: {envelope}"
    );
    assert_eq!(
        checkouts_named(&fixture, &job, "candidate-").await,
        1,
        "the owner prepared the candidate with no children in play"
    );
}

#[tokio::test]
async fn a_ready_candidate_carries_no_preparation_reason() {
    // Mode 75: the owner prepares a ready candidate, so the view names no
    // reason at all — the key is absent, not empty.
    let (_fixture, _job, _provider, output) = run_direct_prepare_output(
        "direct-finish-no-reason",
        75,
        vec!["added.txt".into()],
        vec!["grep".into()],
    )
    .await;
    let envelope = final_envelope(&output);
    assert_eq!(envelope["candidate_ready"], true, "{envelope}");
    assert!(
        envelope["readiness"].get("preparation").is_none(),
        "a ready candidate names no preparation reason: {envelope}"
    );
}
