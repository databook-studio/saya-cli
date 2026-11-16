//! The edit-time in-place baseline: a root is stated, not walked. A tree far
//! over the snapshot file bound starts, special files and an escaping
//! credential-shaped symlink are not the job's business, and the job edits
//! through its own before-image store.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingJobRequest, NativeCodingJob, dispatch_coding,
};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::Value;
use std::{ffi::CString, fs, path::PathBuf, sync::Arc, time::Duration};

const RUN: Duration = Duration::from_secs(60);

fn records(fixture: &Fixture, id: &str) -> PathBuf {
    fixture.root.join("coding").join(id)
}

/// The checkout id the job's private record names, read from disk so a test
/// that started the job through the command surface can review it.
fn recorded_candidate(fixture: &Fixture, id: &str) -> ManagedCheckoutId {
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

/// Proposes and creates a job over the fixture's folder, without running it.
async fn create_write(fixture: &Fixture, id: &str, paths: &[&str]) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "edit the folder".into(),
        paths.iter().map(|path| (*path).to_owned()).collect(),
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

/// Runs the in-place edit script (read, edit twice, create, delete) end to end.
async fn run_edit_script(fixture: &Fixture, id: &str) {
    let command = CodingCommand::Start {
        id: id.into(),
        task: "edit, create and delete in the plain folder".into(),
        write_paths: vec![".".into()],
        database_profiles: vec![],
        tools: vec![
            "read".into(),
            "edit".into(),
            "write".into(),
            "delete".into(),
        ],
        plan: false,
        approve: true,
        in_place: true,
        managed: false,
    };
    tokio::time::timeout(RUN, dispatch_coding(&fixture.application, command))
        .await
        .expect("the scripted run terminates")
        .unwrap();
}

/// Reviews a job that was started through the command surface.
async fn review_job(fixture: &Fixture, id: &str) -> Value {
    let candidate = recorded_candidate(fixture, id);
    dispatch_coding(
        &fixture.application,
        CodingCommand::ReviewExisting {
            id: id.into(),
            candidate: candidate.as_str().into(),
            approve: true,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{id} is reviewable: {error}"))
}

/// A tree past the snapshot file bound is not inventoried at all: the job
/// starts and reviews against an edit-time marker.
#[tokio::test]
async fn in_place_large_tree() {
    let fixture = Fixture::in_place("in-place-large-tree").await;
    let tree = fixture.repository.join("node_modules");
    fs::create_dir(&tree).unwrap();
    for index in 0..50_001 {
        fs::write(tree.join(format!("pkg-{index:05}.js")), b"x\n").unwrap();
    }
    let job = create_write(&fixture, "job-in-place-large-tree", &["."]).await;
    let candidate = recorded_candidate(&fixture, "job-in-place-large-tree");
    let review = job
        .review_existing(&candidate)
        .await
        .expect("a 50,001-file tree starts and reviews");
    assert_eq!(
        review.preview()["baseline"]["kind"],
        "edit_time",
        "{}",
        review.preview()
    );
    assert_eq!(
        review.preview()["changes"],
        serde_json::json!([]),
        "{}",
        review.preview()
    );
}

/// A `.git` file, a submodule-style pointer, a FIFO and a socket are ordinary
/// folder content: none is classified, so none refuses the job.
#[tokio::test]
async fn in_place_special_files() {
    let provider = Provider::before_images();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-special-files", &provider.endpoint).await;
    fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    fs::write(
        fixture.repository.join(".git"),
        b"gitdir: /nonexistent/worktree\n",
    )
    .unwrap();
    fs::create_dir(fixture.repository.join("sub")).unwrap();
    fs::write(
        fixture.repository.join("sub/.git"),
        b"gitdir: ../.git/modules/sub\n",
    )
    .unwrap();
    let fifo = CString::new(fixture.repository.join("pipe").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    // A socket path is limited to SUN_LEN, shorter than the fixture's root:
    // bind it at a short name and move the socket file into the folder.
    let short = PathBuf::from(format!(
        "/tmp/saya-p4b2-{}-{}.sock",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _socket = std::os::unix::net::UnixListener::bind(&short).unwrap();
    fs::rename(&short, fixture.repository.join("sock")).unwrap();

    run_edit_script(&fixture, "in-place-special-files").await;

    let status = dispatch_coding(
        &fixture.application,
        CodingCommand::Show {
            id: "in-place-special-files".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    let review = review_job(&fixture, "in-place-special-files").await;
    let paths = review["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(paths.contains(&"a.txt"), "{review}");
    assert!(paths.contains(&"new.txt"), "{review}");
    assert!(paths.contains(&"old.txt"), "{review}");
}

/// K1's H2 intent: a credential-shaped symlink that points out of the root is
/// never followed and never refuses the job.
#[tokio::test]
async fn in_place_escaped_sensitive_symlink_starts_and_reviews() {
    let provider = Provider::before_images();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-sensitive-link", &provider.endpoint).await;
    fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    fs::create_dir(fixture.repository.join("conf")).unwrap();
    std::os::unix::fs::symlink("../..", fixture.repository.join("conf/.env")).unwrap();

    run_edit_script(&fixture, "in-place-sensitive-link").await;

    let review = review_job(&fixture, "in-place-sensitive-link").await;
    assert_eq!(review["mode"], "in_place", "{review}");
    assert!(
        review["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|change| change["path"] == "a.txt"),
        "{review}"
    );
}
