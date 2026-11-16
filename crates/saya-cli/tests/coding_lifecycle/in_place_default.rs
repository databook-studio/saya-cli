//! The default root mode: a Git repository starts in place unless the launch
//! states `--managed`, a managed job keeps its checkout, and a listing sees
//! both kinds of job in one repository.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingCommand, CodingJobRequest, RootMode, dispatch_coding};
use saya_config::JobRootMode;
use saya_types::JobId;
use serde_json::Value;
use std::{fs, path::PathBuf, sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn start(id: &str, in_place: bool, managed: bool) -> CodingCommand {
    CodingCommand::Start {
        id: id.into(),
        task: "edit, create and delete in the folder".into(),
        write_paths: vec![".".into()],
        database_profiles: Vec::new(),
        tools: vec![
            "read".into(),
            "edit".into(),
            "write".into(),
            "delete".into(),
        ],
        plan: false,
        approve: true,
        in_place,
        managed,
    }
}

fn state_root(fixture: &Fixture) -> PathBuf {
    fixture.root.join("coding")
}

/// The application the CLI opens for `command`: the mode comes from the
/// command's flag, then the configured default, then the job's record.
async fn open_for(fixture: &Fixture, command: &CodingCommand) -> Arc<CodingApplication> {
    let state_root = state_root(fixture);
    CodingApplication::open_with_mode(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &state_root,
        &fixture.git,
        &fixture.program_dir,
        command.root_mode_for(&state_root, fixture.runtime.resolved.coding.root_mode),
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

fn seed(fixture: &Fixture) {
    fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    for args in [
        &["add", "a.txt", "old.txt"][..],
        &["commit", "--quiet", "-m", "editable files"],
    ] {
        let output = std::process::Command::new(&fixture.git)
            .current_dir(&fixture.repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

async fn run(fixture: &Fixture, command: CodingCommand) -> Value {
    let app = open_for(fixture, &command).await;
    tokio::time::timeout(RUN, dispatch_coding(&app, command))
        .await
        .expect("the scripted run terminates")
        .unwrap()
}

/// A Git repository with no launch flag works in place: the configured
/// default, no managed checkout, and the job's edits land in the tree.
#[tokio::test]
async fn start_in_a_git_repo_defaults_to_in_place() {
    let provider = Provider::before_images();
    let fixture = Fixture::with_endpoint("default-in-place", Some(&provider.endpoint)).await;
    seed(&fixture);
    let command = start("job-default-in-place", false, false);
    let state_root = state_root(&fixture);
    assert_eq!(
        command.root_mode_for(&state_root, fixture.runtime.resolved.coding.root_mode),
        RootMode::InPlace,
        "no flag follows the in_place default"
    );

    let result = run(&fixture, command).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert!(
        !state_root
            .join("job-default-in-place/root-checkout.json")
            .exists(),
        "no managed checkout was created"
    );
    assert_eq!(
        fs::read_to_string(fixture.repository.join("a.txt")).unwrap(),
        "gamma\n",
        "the edits landed in the repository tree"
    );
    assert!(fixture.repository.join("new.txt").is_file());
    assert!(!fixture.repository.join("old.txt").exists());
}

/// `--managed` keeps today's behaviour: a checkout, a managed proposal, and a
/// repository tree the job never touches.
#[tokio::test]
async fn managed_flag_keeps_checkout() {
    let provider = Provider::before_images();
    let fixture = Fixture::with_endpoint("managed-flag", Some(&provider.endpoint)).await;
    seed(&fixture);
    let command = start("job-managed-flag", false, true);
    let state_root = state_root(&fixture);
    assert_eq!(
        command.root_mode_for(&state_root, JobRootMode::InPlace),
        RootMode::Managed,
        "--managed beats an in_place config"
    );

    let app = open_for(&fixture, &command).await;
    let request = CodingJobRequest::new(
        JobId::parse("job-managed-flag").unwrap(),
        "edit, create and delete in the folder".into(),
        vec![".".into()],
    )
    .unwrap();
    let proposal = app.propose(&request).await.unwrap();
    assert_ne!(
        proposal.request()["mode"].as_str(),
        Some("in_place"),
        "a managed proposal keeps its managed shape: {:?}",
        proposal.request()
    );

    let result = tokio::time::timeout(RUN, dispatch_coding(&app, command))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    assert_eq!(result["root_mode"], "managed", "{result}");
    assert!(
        state_root
            .join("job-managed-flag/root-checkout.json")
            .exists(),
        "the managed checkout record exists"
    );
    assert_eq!(
        fs::read_to_string(fixture.repository.join("a.txt")).unwrap(),
        "alpha\n",
        "a managed job leaves the repository tree untouched"
    );

    let show = CodingCommand::Show {
        id: "job-managed-flag".into(),
    };
    let show_app = open_for(&fixture, &show).await;
    let status = dispatch_coding(&show_app, show).await.unwrap();
    assert_eq!(status["root_mode"], "managed", "{status}");
}

/// One managed and one in-place job in the same repository are both listed
/// from the in-place root, under the folder id and the repository id.
#[tokio::test]
async fn list_shows_both_modes() {
    let provider = Provider::before_images();
    let fixture = Fixture::with_endpoint("list-both-modes", Some(&provider.endpoint)).await;
    seed(&fixture);

    run(&fixture, start("job-managed-listed", false, true)).await;
    let in_place_app = {
        let command = start("job-in-place-listed", false, false);
        let app = open_for(&fixture, &command).await;
        tokio::time::timeout(RUN, dispatch_coding(&app, command))
            .await
            .expect("the scripted run terminates")
            .unwrap();
        app
    };

    let listed = in_place_app.list(None).await.unwrap();
    let ids: Vec<&str> = listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|job| job["job_id"].as_str())
        .collect();
    assert!(
        ids.contains(&"job-managed-listed") && ids.contains(&"job-in-place-listed"),
        "both root modes are listed: {listed}"
    );
}
