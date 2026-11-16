//! `saya job show` for an in-place job: the lifecycle verb resolves the root
//! mode from the job's private record, and the report states the baseline,
//! the warnings and the changes finalize recorded against it.

use super::{provider::Provider, support::Fixture};
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingCommand, dispatch_coding};
use saya_types::ManagedCheckoutId;
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn state_root(fixture: &Fixture) -> PathBuf {
    fixture.root.join("coding")
}

fn records(fixture: &Fixture, id: &str) -> PathBuf {
    state_root(fixture).join(id)
}

/// The application a lifecycle verb opens: the same repository and state
/// root, with the root mode the command resolves.
async fn reopened(fixture: &Fixture, command: &CodingCommand) -> Arc<CodingApplication> {
    CodingApplication::open_with_mode(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &state_root(fixture),
        &fixture.git,
        &fixture.program_dir,
        command.root_mode_for(
            &state_root(fixture),
            fixture.runtime.resolved.coding.root_mode,
        ),
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

async fn start(fixture: &Fixture, id: &str) -> Value {
    let command = CodingCommand::Start {
        id: id.into(),
        task: "write the two assigned files".into(),
        write_paths: vec![".".into()],
        database_profiles: vec![],
        tools: vec![],
        plan: false,
        approve: true,
        in_place: true,
        managed: false,
    };
    tokio::time::timeout(RUN, dispatch_coding(&fixture.application, command))
        .await
        .expect("the scripted run terminates")
        .unwrap()
}

fn candidate(fixture: &Fixture, id: &str) -> ManagedCheckoutId {
    let record: Value =
        serde_json::from_slice(&fs::read(records(fixture, id).join("inplace-root.json")).unwrap())
            .unwrap();
    let root_id = record["root_id"].as_str().unwrap();
    ManagedCheckoutId::parse(&format!(
        "inplace-{}",
        root_id.strip_prefix("loc-").unwrap()
    ))
    .unwrap()
}

fn changes(value: &Value) -> Vec<(String, String)> {
    value["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_owned(),
                entry["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn expected() -> Vec<(String, String)> {
    vec![
        ("turn-one.txt".to_owned(), "created".to_owned()),
        ("turn-two.txt".to_owned(), "created".to_owned()),
    ]
}

/// `saya job show` for the job: state, outcome, baseline summary, warnings,
/// and the bounded changes finalize recorded.
async fn assert_show(fixture: &Fixture, id: &str) -> Value {
    let command = CodingCommand::Show { id: id.into() };
    let app = reopened(fixture, &command).await;
    let status = dispatch_coding(&app, command).await.unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["mode"], "in_place", "{status}");
    assert_eq!(status["root_mode"], "in_place", "{status}");
    assert_eq!(status["outcome"]["state"], "completed", "{status}");
    assert!(
        status["snapshot_warnings"].as_array().unwrap().is_empty(),
        "in-place omissions are not included-sensitive warnings: {status}"
    );
    assert_eq!(
        status["baseline"]["kind"], "edit_time",
        "the baseline is an edit-time marker: {status}"
    );
    assert!(
        status["baseline"].get("files").is_none(),
        "an edit-time baseline holds no file inventory: {status}"
    );
    assert_eq!(
        status["recorded"]["files"], 2,
        "the before-image store names the two tool-created files: {status}"
    );
    assert_eq!(
        status["recorded"]["bytes"], 18,
        "the before-image store names the recorded bytes: {status}"
    );
    assert_eq!(
        status["recorded"]["overflowed"], false,
        "no before-image bound was reached: {status}"
    );
    assert_eq!(status["changed_paths_truncated"], false, "{status}");
    assert_eq!(changes(&status), expected(), "{status}");
    assert_eq!(
        status["outcome"]["changes"]["files"], 2,
        "the outcome carries the same change count: {status}"
    );
    status
}

/// The start result states the same in-place edits the show does.
async fn assert_start_reports_changes(fixture: &Fixture, id: &str) -> Value {
    let started = start(fixture, id).await;
    assert_eq!(started["outcome"]["state"], "completed", "{started}");
    assert_eq!(
        changes(&started["outcome"]),
        expected(),
        "the start result reports the in-place edits: {started}"
    );
    started
}

async fn finalize(fixture: &Fixture, id: &str) -> Value {
    let candidate = candidate(fixture, id);
    let review_command = CodingCommand::ReviewExisting {
        id: id.into(),
        candidate: candidate.as_str().into(),
        approve: true,
    };
    let app = reopened(fixture, &review_command).await;
    let review = dispatch_coding(&app, review_command).await.unwrap();
    let source_sha256 = review["source_sha256"].as_str().unwrap().to_owned();
    let apply = CodingCommand::Apply {
        id: id.into(),
        candidate: candidate.as_str().into(),
        source_sha256,
        approve: true,
    };
    let applied = dispatch_coding(&app, apply).await.unwrap();
    assert_eq!(applied["mode"], "in_place", "{applied}");
    assert_eq!(applied["already_applied"], false, "{applied}");
    let show = CodingCommand::Show { id: id.into() };
    dispatch_coding(&app, show).await.unwrap()
}

#[tokio::test]
async fn in_place_job_in_a_git_worktree_reports_changes_and_shows() {
    let provider = Provider::script(false, 113);
    let fixture =
        Fixture::in_place_git_with_endpoint("in-place-show-git", &provider.endpoint).await;
    let id = "in-place-show-git";
    assert_start_reports_changes(&fixture, id).await;
    let status = assert_show(&fixture, id).await;
    let shown = finalize(&fixture, id).await;
    assert_eq!(shown["finalized"], true, "{shown}");
    assert_eq!(
        changes(&shown),
        changes(&status),
        "finalize acknowledges the reviewed folder without changing the report"
    );
}

#[tokio::test]
async fn in_place_job_in_a_plain_folder_reports_changes_and_shows() {
    let provider = Provider::script(false, 113);
    let fixture = Fixture::in_place_with_endpoint("in-place-show-plain", &provider.endpoint).await;
    let id = "in-place-show-plain";
    assert_start_reports_changes(&fixture, id).await;
    let status = assert_show(&fixture, id).await;
    let shown = finalize(&fixture, id).await;
    assert_eq!(shown["finalized"], true, "{shown}");
    assert_eq!(changes(&shown), changes(&status));
}
