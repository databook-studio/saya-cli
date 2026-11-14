//! `saya start`, `saya job resume`, and the TUI `/start` panel share one
//! consent rule: an edit-capable mode implies approval and defaults an empty
//! write scope to the folder; `ask` still needs `--approve`.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{ApprovalPolicy, CancellationToken, NoopEventSink};
use saya_cli::{
    CodingApplication, CodingCommand, CodingError, OneShotMode, OneShotRequest, dispatch_coding,
    launch_plan, oneshot_start,
};
use saya_types::JobId;
use std::{sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn start(id: &str, task: &str, write_paths: Vec<String>, approve: bool) -> CodingCommand {
    CodingCommand::Start {
        id: id.into(),
        task: task.into(),
        write_paths,
        database_profiles: Vec::new(),
        tools: Vec::new(),
        plan: false,
        approve,
        in_place: true,
        managed: false,
    }
}

/// The application `saya start` opens, with the consent the session stated.
async fn application_with(fixture: &Fixture, policy: ApprovalPolicy) -> Arc<CodingApplication> {
    CodingApplication::open_in_place(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .expect("the in-place application opens")
    .with_approval_policy(policy)
}

#[tokio::test]
async fn start_under_bypass_needs_no_approve_flag() {
    let provider = Provider::answer_only();
    let fixture =
        Fixture::in_place_git_with_endpoint("start-bypass-consent", &provider.endpoint).await;
    let application = application_with(&fixture, ApprovalPolicy::Bypass).await;

    let result = tokio::time::timeout(
        RUN,
        dispatch_coding(
            &application,
            start("start-bypass-consent", "answer only", Vec::new(), false),
        ),
    )
    .await
    .expect("the scripted run terminates")
    .expect("bypass is the consent: no --approve is needed");

    assert_eq!(result["job_id"], "start-bypass-consent");
    let status = application
        .status(&JobId::parse("start-bypass-consent").unwrap())
        .await
        .unwrap();
    assert_eq!(status["state"], "completed", "{status}");
}

#[tokio::test]
async fn start_under_accept_edits_defaults_the_write_scope_to_the_folder() {
    let provider = Provider::before_images();
    let fixture =
        Fixture::in_place_git_with_endpoint("start-accept-edits", &provider.endpoint).await;
    std::fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    std::fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    let application = application_with(&fixture, ApprovalPolicy::AcceptEdits).await;

    let result = tokio::time::timeout(
        RUN,
        dispatch_coding(
            &application,
            start(
                "start-accept-edits",
                "edit the plain folder",
                Vec::new(),
                false,
            ),
        ),
    )
    .await
    .expect("the scripted run terminates")
    .expect("accept-edits is the consent: no --approve is needed");

    assert!(
        fixture.repository.join("new.txt").exists(),
        "an empty --write scope means the folder: {result}"
    );
    let changed = result["outcome"]["changed_paths"]
        .as_array()
        .expect("the outcome names its changed paths");
    assert!(
        changed.iter().any(|entry| entry["path"] == "new.txt"),
        "{changed:?}"
    );
}

#[tokio::test]
async fn start_under_ask_still_needs_approve() {
    let provider = Provider::answer_only();
    let fixture = Fixture::in_place_git_with_endpoint("start-ask", &provider.endpoint).await;

    let refusal = dispatch_coding(
        &fixture.application,
        start("start-ask", "answer only", vec![".".into()], false),
    )
    .await
    .expect_err("ask still asks: --approve is required");

    assert!(
        matches!(refusal, CodingError::ApprovalRequired),
        "the refusal is the approve gate: {refusal}"
    );
}

#[tokio::test]
async fn resume_returns_the_result_object_and_exit_code() {
    let provider = Provider::answer_only();
    let fixture = Fixture::in_place_git_with_endpoint("resume-result", &provider.endpoint).await;
    let plan = launch_plan(OneShotMode::ReadOnly, &[], false).expect("read-only plans");
    let job = oneshot_start(
        &fixture.application,
        OneShotRequest {
            task: "answer only".into(),
            job_id: Some("resume-result".into()),
            write_paths: Vec::new(),
            mode: OneShotMode::ReadOnly,
        },
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("the job starts");
    job.run(&NoopEventSink).await.expect("the job completes");
    // The in-place lease belongs to the live handle: release it before the
    // resume opens its own.
    drop(job);

    let result = dispatch_coding(
        &fixture.application,
        CodingCommand::Resume {
            id: "resume-result".into(),
            approve: true,
        },
    )
    .await
    .expect("the resume runs");

    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["subtype"], "success", "{result}");
    assert_eq!(result["session_id"], "resume-result", "{result}");
    assert_eq!(result["job_id"], "resume-result", "{result}");
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["result"], "fallback start answered", "{result}");
}
