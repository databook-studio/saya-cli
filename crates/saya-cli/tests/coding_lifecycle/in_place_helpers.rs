//! A helper of an in-place job works in the same folder as its root, under the
//! root's lease and through the job's one before-image store, so the helper's
//! edits are reported by `job show` and `job review` exactly as the root's.

use super::provider::Provider;
use super::support::Fixture;
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

/// The application a command opens over: the root mode is the command's, then
/// the configured default, then the job's own record.
async fn opened(fixture: &Fixture, command: &CodingCommand) -> Arc<CodingApplication> {
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

/// The synthetic checkout id an in-place root names: derived from the folder
/// id, so the root and its helpers share it and it never names a checkout row.
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

fn changed_paths(value: &Value) -> Vec<String> {
    value["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str().map(str::to_owned))
        .collect()
}

/// A Git working tree with no root flag runs in place; its scripted root
/// delegates one helper, which writes its own file in the same folder, and
/// both edits are reported by the job's show and review.
#[tokio::test]
async fn an_in_place_job_runs_a_helper_and_reports_its_edits() {
    // Mode 62: the root hands `file-a.txt` to one helper, then writes
    // `file-b.txt` itself and answers.
    let provider = Provider::script(false, 62);
    let fixture = Fixture::in_place_git_with_endpoint("in-place-helper", &provider.endpoint).await;
    let command = CodingCommand::Start {
        id: "job-in-place-helper".into(),
        task: "delegate one file and write another".into(),
        write_paths: vec![".".into()],
        database_profiles: vec![],
        tools: vec!["helpers".into(), "write".into()],
        plan: false,
        approve: true,
        in_place: false,
        managed: false,
    };
    let app = opened(&fixture, &command).await;
    let started = tokio::time::timeout(RUN, dispatch_coding(&app, command))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    assert_eq!(started["root_mode"], "in_place", "{started}");
    assert_eq!(started["outcome"]["state"], "completed", "{started}");

    // The root's `helpers` call returned the helper's own completed summary.
    let summary = &super::direct_task::tool_result(&provider, "call-task")[0];
    assert_eq!(summary["outcome"], "completed", "{summary}");

    // The helper's write landed in the user's own folder, beside the root's.
    assert_eq!(
        fs::read_to_string(fixture.repository.join("file-a.txt")).unwrap(),
        "child-a wrote\n",
        "the helper's edit is in the root's folder"
    );
    assert_eq!(
        fs::read_to_string(fixture.repository.join("file-b.txt")).unwrap(),
        "root wrote after the merge\n",
        "the root's edit is in the same folder"
    );

    // `job show` reports the helper's edit as part of the job's changes.
    let show = CodingCommand::Show {
        id: "job-in-place-helper".into(),
    };
    let show_app = opened(&fixture, &show).await;
    let status = dispatch_coding(&show_app, show).await.unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["root_mode"], "in_place", "{status}");
    let shown = changed_paths(&status);
    assert!(
        shown.contains(&"file-a.txt".to_owned()) && shown.contains(&"file-b.txt".to_owned()),
        "job show reports the helper's edit and the root's: {status}"
    );

    // `job review` reads the same before-image store, so the helper's edit is
    // part of the reviewed change set.
    let review = CodingCommand::ReviewExisting {
        id: "job-in-place-helper".into(),
        candidate: candidate(&fixture, "job-in-place-helper").as_str().into(),
        approve: true,
    };
    let review_app = opened(&fixture, &review).await;
    let reviewed = dispatch_coding(&review_app, review).await.unwrap();
    assert_eq!(reviewed["mode"], "in_place", "{reviewed}");
    let changes: Vec<&str> = reviewed["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|change| change["path"].as_str())
        .collect();
    assert!(
        changes.contains(&"file-a.txt") && changes.contains(&"file-b.txt"),
        "job review reports the helper's edit and the root's: {reviewed}"
    );
}
