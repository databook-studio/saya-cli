//! A managed start that cannot use a checkout falls back to the folder in
//! place, states one reason, and records it on the job.

use super::{provider::Provider, support::Fixture};
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingCommand, CodingJobRequest, RootMode, dispatch_coding};
use saya_types::JobId;
use serde_json::Value;
use std::{fs, path::PathBuf, process::Command, sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn state_root(fixture: &Fixture) -> PathBuf {
    fixture.root.join("coding")
}

fn records(fixture: &Fixture, id: &str) -> PathBuf {
    state_root(fixture).join(id)
}

fn start(id: &str, managed: bool) -> CodingCommand {
    CodingCommand::Start {
        id: id.into(),
        task: "resolve the conflicted file".into(),
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
        in_place: false,
        managed,
    }
}

fn git(fixture: &Fixture, args: &[&str]) {
    let output = Command::new(&fixture.git)
        .current_dir(&fixture.repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn open(fixture: &Fixture, mode: RootMode) -> (Arc<CodingApplication>, Option<&'static str>) {
    CodingApplication::open_for_start(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &state_root(fixture),
        &fixture.git,
        &fixture.program_dir,
        mode,
        &CancellationToken::new(),
    )
    .await
    .unwrap()
}

/// Runs the start over the mode the command resolves. When the open itself
/// fell back, the result must state that same reason.
async fn run(fixture: &Fixture, command: CodingCommand) -> Value {
    let mode = command.root_mode_for(
        &state_root(fixture),
        fixture.runtime.resolved.coding.root_mode,
    );
    let (application, opened) = open(fixture, mode).await;
    let result = tokio::time::timeout(RUN, dispatch_coding(&application, command))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    if let Some(reason) = opened {
        assert_eq!(result["managed_fallback"]["reason"], reason, "{result}");
    }
    result
}

/// Leaves `tracked.txt` mid-merge with Git's conflict markers.
fn conflict(fixture: &Fixture) {
    let branch = Command::new(&fixture.git)
        .current_dir(&fixture.repository)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .unwrap();
    assert!(branch.status.success());
    let branch = String::from_utf8(branch.stdout).unwrap();
    let branch = branch.trim();
    git(fixture, &["checkout", "--quiet", "-b", "other"]);
    fs::write(fixture.repository.join("tracked.txt"), b"theirs\n").unwrap();
    git(fixture, &["commit", "--quiet", "-am", "theirs"]);
    git(fixture, &["checkout", "--quiet", branch]);
    fs::write(fixture.repository.join("tracked.txt"), b"ours\n").unwrap();
    git(fixture, &["commit", "--quiet", "-am", "ours"]);
    let merge = Command::new(&fixture.git)
        .current_dir(&fixture.repository)
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge conflicts");
    assert_eq!(
        fs::read_to_string(fixture.repository.join("tracked.txt")).unwrap(),
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\n"
    );
}

fn changed_paths(status: &Value) -> Vec<&str> {
    status["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect()
}

#[tokio::test]
async fn managed_fallback_conflicted_index() {
    let provider = Provider::resolve_conflict();
    let fixture =
        Fixture::with_endpoint("fallback-conflicted-index", Some(&provider.endpoint)).await;
    conflict(&fixture);

    let result = run(&fixture, start("job-fallback-conflicted-index", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "conflicted_index",
        "{result}"
    );
    assert_eq!(
        fs::read_to_string(fixture.repository.join("tracked.txt")).unwrap(),
        "resolved\n",
        "the in-place edit resolved the file in the repository tree"
    );
    let record: Value = serde_json::from_slice(
        &fs::read(records(&fixture, "job-fallback-conflicted-index").join("root-mode.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["requested"], "managed");
    assert_eq!(record["effective"], "in_place");
    assert_eq!(record["reason"], "conflicted_index");

    let (application, _) = open(&fixture, RootMode::InPlace).await;
    let show = CodingCommand::Show {
        id: "job-fallback-conflicted-index".into(),
    };
    let status = dispatch_coding(&application, show).await.unwrap();
    assert_eq!(
        status["managed_fallback"]["reason"], "conflicted_index",
        "{status}"
    );
    assert!(
        changed_paths(&status).contains(&"tracked.txt"),
        "the review lists the resolved file: {status}"
    );
}

#[tokio::test]
async fn resolve_merge_conflict_default_mode() {
    let provider = Provider::resolve_conflict();
    let fixture =
        Fixture::with_endpoint("resolve-conflict-default", Some(&provider.endpoint)).await;
    conflict(&fixture);

    let result = run(&fixture, start("job-conflict-default", false)).await;
    assert!(
        result.get("managed_fallback").is_none(),
        "the default mode needs no fallback: {result}"
    );
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        fs::read_to_string(fixture.repository.join("tracked.txt")).unwrap(),
        "resolved\n"
    );
}

#[tokio::test]
async fn managed_fallback_unborn_head() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-unborn-head", Some(&provider.endpoint)).await;
    git(&fixture, &["update-ref", "-d", "HEAD"]);

    let result = run(&fixture, start("job-fallback-unborn", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "unborn_head",
        "{result}"
    );
}

#[tokio::test]
async fn managed_fallback_submodule_recursion() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-submodule", Some(&provider.endpoint)).await;
    git(&fixture, &["config", "submodule.vendor.recurse", "true"]);

    let result = run(&fixture, start("job-fallback-submodule", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "submodule_recursion",
        "{result}"
    );
}

#[tokio::test]
async fn managed_fallback_sparse_checkout() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-sparse", Some(&provider.endpoint)).await;
    git(&fixture, &["config", "core.sparseCheckout", "true"]);

    let result = run(&fixture, start("job-fallback-sparse", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "sparse_checkout",
        "{result}"
    );
}

#[tokio::test]
async fn managed_fallback_lfs_or_filter_attributes() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-lfs-attr", Some(&provider.endpoint)).await;
    fs::write(
        fixture.repository.join(".gitattributes"),
        b"*.txt filter=lfs\n",
    )
    .unwrap();

    let result = run(&fixture, start("job-fallback-lfs-attr", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "lfs_or_filter_attributes",
        "{result}"
    );
}

#[tokio::test]
async fn managed_fallback_nested_repository() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-nested", Some(&provider.endpoint)).await;
    fs::create_dir_all(fixture.repository.join("nested/.git")).unwrap();
    fs::write(
        fixture.repository.join("nested/.git/HEAD"),
        b"ref: refs/heads/main\n",
    )
    .unwrap();
    // Git hides a directory holding `.git` from status, so the outer index
    // must name a path inside it for the inventory to list that directory.
    fs::write(fixture.repository.join("nested/file.txt"), b"nested\n").unwrap();
    git(&fixture, &["add", "-f", "nested/file.txt"]);

    let result = run(&fixture, start("job-fallback-nested", true)).await;
    assert_eq!(result["root_mode"], "in_place", "{result}");
    assert_eq!(
        result["managed_fallback"]["reason"], "nested_repository",
        "{result}"
    );
}

#[tokio::test]
async fn managed_cancel_is_not_fallback() {
    let provider = Provider::answer_only();
    let fixture = Fixture::with_endpoint("fallback-cancelled", Some(&provider.endpoint)).await;
    let (application, opened) = open(&fixture, RootMode::Managed).await;
    assert_eq!(opened, None, "the managed open itself succeeds");
    assert!(!application.is_in_place());

    let token = CancellationToken::new();
    token.cancel();
    let request = CodingJobRequest::new(
        JobId::parse("job-fallback-cancelled").unwrap(),
        "resolve the conflicted file".into(),
        vec![".".into()],
    )
    .unwrap();
    if application.plan_for_start(&request, &token).await.is_ok() {
        panic!("a cancelled plan is an error, not a silent mode switch");
    }
    assert!(
        !application.is_in_place(),
        "a cancelled plan never switches modes"
    );
    assert!(
        !records(&fixture, "job-fallback-cancelled")
            .join("root-mode.json")
            .exists()
    );
}
