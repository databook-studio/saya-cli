//! Shell-made changes in an in-place root: a Git worktree records the status
//! difference from job start and reports it beside the tool changes; a plain
//! folder says it could not record any. Shell changes are reported, never
//! rewound.

use super::provider::Provider;
use super::support::Fixture;
use saya_cli::{CodingCommand, dispatch_coding};
use saya_types::ManagedCheckoutId;
use serde_json::Value;
use std::{fs, path::PathBuf, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn records(fixture: &Fixture, id: &str) -> PathBuf {
    fixture.root.join("coding").join(id)
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

async fn review(fixture: &Fixture, id: &str) -> Value {
    let candidate = candidate(fixture, id);
    dispatch_coding(
        &fixture.application,
        CodingCommand::ReviewExisting {
            id: id.into(),
            candidate: candidate.as_str().into(),
            approve: true,
        },
    )
    .await
    .unwrap()
}

fn change_paths(value: &Value) -> Vec<String> {
    value["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["path"].as_str().unwrap().to_owned())
        .collect()
}

fn shell_paths(value: &Value) -> Vec<(String, String)> {
    value["shell_changes"]["paths"]
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

#[tokio::test]
async fn in_place_review_reports_shell_changes_git() {
    let provider = Provider::script(false, 113);
    let fixture =
        Fixture::in_place_git_with_endpoint("in-place-shell-git", &provider.endpoint).await;
    let started = start(&fixture, "in-place-shell-git").await;
    assert_eq!(started["outcome"]["state"], "completed", "{started}");
    // The user's own shell, after the job: a new file and an edit to the
    // tracked file that was clean at start.
    fs::write(fixture.repository.join("shell.txt"), b"shell\n").unwrap();
    fs::write(fixture.repository.join("tracked.txt"), b"shell changed\n").unwrap();

    let review = review(&fixture, "in-place-shell-git").await;
    assert_eq!(review["shell_changes"]["status"], "recorded", "{review}");
    assert_eq!(
        review["shell_changes"]["commits_since_start"], 0,
        "{review}"
    );
    assert_eq!(
        shell_paths(&review),
        vec![
            ("shell.txt".to_owned(), "added".to_owned()),
            ("tracked.txt".to_owned(), "modified".to_owned()),
        ],
        "{review}"
    );
    assert_eq!(
        change_paths(&review),
        vec!["turn-one.txt", "turn-two.txt"],
        "the tool changes stay the job's own: {review}"
    );
    assert!(
        review["note"]
            .as_str()
            .is_some_and(|note| note.contains("never rewound")),
        "{review}"
    );
}

#[tokio::test]
async fn in_place_review_plain_folder_unrecorded() {
    let provider = Provider::script(false, 113);
    let fixture = Fixture::in_place_with_endpoint("in-place-shell-plain", &provider.endpoint).await;
    start(&fixture, "in-place-shell-plain").await;
    fs::write(fixture.repository.join("shell.txt"), b"shell\n").unwrap();

    let review = review(&fixture, "in-place-shell-plain").await;
    assert_eq!(
        review["shell_changes"],
        serde_json::json!({"status": "unrecorded", "reason": "not_a_git_repository"}),
        "{review}"
    );
    assert_eq!(
        change_paths(&review),
        vec!["turn-one.txt", "turn-two.txt"],
        "{review}"
    );
}
