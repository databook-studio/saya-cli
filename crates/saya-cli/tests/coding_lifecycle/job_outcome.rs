//! The job outcome block: a run that ends with a final answer is `completed`
//! and its result carries that answer, the commits it made in the working
//! checkout, the paths it changed and the checkout itself. A guard stop is a
//! real pause: `paused`, with the guard named.

use super::direct_task::{direct_job, root_checkout};
use super::support::Fixture;
use saya_agent::NoopEventSink;
use saya_types::JobState;
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// The outcome block `saya start` and `saya job show` both report.
async fn outcome(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> Value {
    fixture.application.status(job.job_id()).await.unwrap()["outcome"].clone()
}

async fn state(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> JobState {
    fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .expect("the job record exists")
        .state
}

/// Runs one git command in the working checkout and returns its trimmed
/// stdout. The fixture's git is the trusted executable the application uses.
fn git(fixture: &Fixture, working: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new(&fixture.git)
        .arg("-C")
        .arg(working)
        .args(arguments)
        .output()
        .expect("git runs in the working checkout");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Waits until the scripted provider has read `count` root requests, bounded by
/// the fixture's event ceiling.
async fn wait_for_calls(provider: &super::provider::Provider, count: usize) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while provider.calls.load(Ordering::Acquire) < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the scripted provider reaches the held response");
}

#[tokio::test]
async fn an_answer_without_edits_completes_and_carries_the_answer() {
    // Mode 76: the root reads twice and answers without changing anything. No
    // commit and no file change still ends `completed`, with the model's own
    // final message as the answer.
    let (fixture, _application, job, _provider) =
        direct_job("outcome-answer", 76, vec!["added.txt".into()], vec![]).await;
    job.run(&NoopEventSink).await.expect("the run finishes");
    assert_eq!(state(&fixture, &job).await, JobState::Completed);
    let outcome = outcome(&fixture, &job).await;
    assert_eq!(outcome["state"], "completed", "{outcome}");
    assert_eq!(
        outcome["answer"], "the direct root changed nothing",
        "the result carries the model's final answer: {outcome}"
    );
    assert_eq!(outcome["answer_truncated"], false, "{outcome}");
    assert_eq!(outcome["commits"], serde_json::json!([]), "{outcome}");
    assert_eq!(
        outcome["changed_paths"],
        serde_json::json!([]),
        "nothing changed: {outcome}"
    );
    assert!(
        outcome["checkout"]
            .as_str()
            .is_some_and(|path| Path::new(path).exists()),
        "the outcome names the working checkout: {outcome}"
    );
    assert_eq!(
        outcome["readiness"]["candidate_ready"], false,
        "readiness is one field: {outcome}"
    );
    assert_eq!(outcome["model_prose"], "untrusted", "{outcome}");
}

#[tokio::test]
async fn a_commit_completes_and_names_the_commit() {
    // Mode 100 holds the parent's final answer until the test releases it, so
    // the test can commit in the working checkout before the run finalizes.
    let (fixture, _application, job, provider) =
        direct_job("outcome-commit", 100, vec!["tracked.txt".into()], vec![]).await;
    let running = Arc::clone(&job);
    let run = tokio::spawn(async move { running.run(&NoopEventSink).await });
    wait_for_calls(&provider, 3).await;
    let working = root_checkout(&fixture, &job).await;
    std::fs::write(working.join("committed.txt"), "committed\n").unwrap();
    git(&fixture, &working, &["add", "committed.txt"]);
    git(
        &fixture,
        &working,
        &["commit", "--quiet", "-m", "job commit"],
    );
    let sha = git(&fixture, &working, &["rev-parse", "HEAD"]);
    provider.release();
    run.await.unwrap().expect("the run finishes");

    assert_eq!(state(&fixture, &job).await, JobState::Completed);
    let outcome = outcome(&fixture, &job).await;
    assert_eq!(outcome["state"], "completed", "{outcome}");
    assert_eq!(outcome["commits"].as_array().unwrap().len(), 1, "{outcome}");
    assert_eq!(outcome["commits"][0]["sha"], sha, "{outcome}");
    assert_eq!(outcome["commits"][0]["subject"], "job commit", "{outcome}");
    assert_eq!(outcome["commits_truncated"], false, "{outcome}");
    assert!(
        outcome["changed_paths"].is_array(),
        "the changed paths are carried even when a commit moved the checkout: {outcome}"
    );
    assert_eq!(
        outcome["checkout"].as_str(),
        Some(working.to_str().unwrap()),
        "the outcome names the working checkout the commit moved"
    );
}

#[tokio::test]
async fn a_guard_stop_pauses_and_names_the_guard() {
    // Mode 115 repeats one read until the repeat guard stops the run. That is
    // a real pause, not a completed job, and the outcome names the guard.
    let (fixture, _application, job, _provider) =
        direct_job("outcome-guard", 115, vec!["tracked.txt".into()], vec![]).await;
    job.run(&NoopEventSink)
        .await
        .expect("the guard stop is clean");
    assert_eq!(state(&fixture, &job).await, JobState::Paused);
    let outcome = outcome(&fixture, &job).await;
    assert_eq!(outcome["state"], "paused", "{outcome}");
    assert_eq!(outcome["reason"], "repeating", "{outcome}");
    assert_eq!(
        outcome["answer"], "the repeated reads stopped the run",
        "the model's last answer survives a guard stop: {outcome}"
    );
}
