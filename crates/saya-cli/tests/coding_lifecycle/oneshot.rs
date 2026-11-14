//! `saya -p`'s library operation, end to end: one in-place job, one result
//! object, and the exit code its state states.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingApplication, OneShotMode, OneShotRequest, launch_plan, oneshot_finish, oneshot_start,
};
use serde_json::json;
use std::time::Instant;

fn request(id: &str, task: &str, mode: OneShotMode, write_paths: Vec<String>) -> OneShotRequest {
    OneShotRequest {
        task: task.to_owned(),
        job_id: Some(id.to_owned()),
        write_paths,
        mode,
    }
}

async fn run(
    application: &std::sync::Arc<CodingApplication>,
    job: &std::sync::Arc<saya_cli::NativeCodingJob>,
) -> saya_cli::OneShotResult {
    let started = Instant::now();
    let result = job.run(&NoopEventSink).await;
    oneshot_finish(application, job, result, started.elapsed()).await
}

#[tokio::test]
async fn one_shot_read_only_completes_and_reports() {
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_git_with_endpoint("one-shot-read-only", &provider.endpoint).await;
    let plan = launch_plan(OneShotMode::ReadOnly, &[], false).expect("read-only plans");
    let job = oneshot_start(
        &fixture.application,
        request(
            "one-shot-read-only",
            "answer after the refused calls",
            OneShotMode::ReadOnly,
            vec![],
        ),
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    let result = run(&fixture.application, &job).await;

    assert_eq!(result.value["subtype"], "success", "{}", result.value);
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        result.value["result"],
        "parity parent finished after the unregistered-child refusals"
    );
    assert_eq!(result.value["session_id"], "one-shot-read-only");
    assert_eq!(result.value["job_id"], "one-shot-read-only");
    assert!(
        result.value["usage"]["requests"].as_u64().unwrap_or(0) >= 1,
        "the usage view counts the provider calls: {}",
        result.value
    );
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    let events = fixture
        .store
        .list_coding_events(job.job_id(), None, 100)
        .await
        .unwrap();
    assert!(!events.events.is_empty(), "the job kept its event stream");
}

#[tokio::test]
async fn one_shot_accept_edits_writes_in_place() {
    let provider = Provider::before_images();
    let fixture =
        Fixture::in_place_git_with_endpoint("one-shot-accept-edits", &provider.endpoint).await;
    std::fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    std::fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    let plan = launch_plan(OneShotMode::AcceptEdits, &[], false).expect("accept-edits plans");
    assert_eq!(plan.write_paths, vec![".".to_owned()]);
    let job = oneshot_start(
        &fixture.application,
        request(
            "one-shot-accept-edits",
            "edit the plain folder",
            OneShotMode::AcceptEdits,
            plan.write_paths.clone(),
        ),
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    let result = run(&fixture.application, &job).await;

    assert!(
        fixture.repository.join("new.txt").exists(),
        "the job wrote in place: {}",
        result.value
    );
    let changed = result.value["outcome"]["changed_paths"]
        .as_array()
        .expect("the outcome names its changed paths");
    assert!(
        changed.iter().any(|entry| entry["path"] == "new.txt"),
        "{changed:?}"
    );
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["checkouts"], json!([]), "{status}");
    let recorded: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            fixture
                .root
                .join("coding/one-shot-accept-edits/request.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recorded["write_paths"],
        json!(["."]),
        "the job records the whole-workspace default"
    );
}

#[tokio::test]
async fn one_shot_explicit_write_scope_denies_a_path_outside_it() {
    let provider = Provider::write_outside_scope();
    let fixture =
        Fixture::in_place_git_with_endpoint("one-shot-scope-denied", &provider.endpoint).await;
    let plan = launch_plan(OneShotMode::AcceptEdits, &["src".to_owned()], false)
        .expect("a stated scope plans");
    let job = oneshot_start(
        &fixture.application,
        request(
            "one-shot-scope-denied",
            "write outside the stated scope",
            OneShotMode::AcceptEdits,
            plan.write_paths.clone(),
        ),
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    let result = run(&fixture.application, &job).await;

    assert_eq!(result.value["subtype"], "success", "{}", result.value);
    assert!(
        !fixture.repository.join("README.md").exists(),
        "a stated scope still refuses a path outside it: {}",
        result.value
    );
    let bodies = provider.bodies.lock().unwrap();
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("it may write src") && body.contains("report what you need")),
        "the refusal tells the model what it may write: {bodies:?}"
    );
}

#[tokio::test]
async fn one_shot_write_outside_the_workspace_is_refused() {
    let provider = Provider::write_outside_workspace();
    let fixture =
        Fixture::in_place_git_with_endpoint("one-shot-workspace-escape", &provider.endpoint).await;
    let plan = launch_plan(OneShotMode::AcceptEdits, &[], false).expect("accept-edits plans");
    let job = oneshot_start(
        &fixture.application,
        request(
            "one-shot-workspace-escape",
            "write outside the workspace",
            OneShotMode::AcceptEdits,
            plan.write_paths.clone(),
        ),
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    let result = run(&fixture.application, &job).await;

    assert_eq!(result.value["subtype"], "success", "{}", result.value);
    assert!(
        !fixture.root.join("escape.txt").exists(),
        "the containment boundary still refuses a path outside the workspace: {}",
        result.value
    );
}

#[tokio::test]
async fn one_shot_turn_backstop_reports_error_max_turns() {
    let provider = Provider::repeated_status();
    let fixture =
        Fixture::in_place_git_with_endpoint("one-shot-max-turns", &provider.endpoint).await;
    let mut runtime = fixture.runtime.clone();
    runtime.resolved.limits.job.logical_requests =
        saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(1).unwrap());
    let application = CodingApplication::open_in_place(
        runtime,
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .expect("the capped application opens in place");
    let plan = launch_plan(OneShotMode::ReadOnly, &[], false).expect("read-only plans");
    let job = oneshot_start(
        &application,
        request(
            "one-shot-max-turns",
            "read until the backstop stops the job",
            OneShotMode::ReadOnly,
            vec![],
        ),
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    let result = run(&application, &job).await;

    assert_eq!(
        result.value["subtype"], "error_max_turns",
        "{}",
        result.value
    );
    assert_eq!(result.value["is_error"], true);
    assert_eq!(result.exit_code, 5);
}
