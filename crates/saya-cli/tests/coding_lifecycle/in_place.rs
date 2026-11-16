//! The in-place stage, end to end: a job over a plain folder runs, answers,
//! creates no checkout, reviews its own folder, and cleans its scratch tree.

use super::provider::Provider;
use super::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, dispatch_coding,
};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Duration};

const RUN: Duration = Duration::from_secs(60);

async fn started(
    fixture: &Fixture,
    id: &str,
    write_paths: Vec<String>,
) -> std::sync::Arc<saya_cli::NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(id).unwrap(),
        "read the plain folder and answer".into(),
        write_paths,
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

async fn run(job: &std::sync::Arc<saya_cli::NativeCodingJob>) {
    tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
}

fn records(fixture: &Fixture, job: &JobId) -> PathBuf {
    fixture.root.join("coding").join(job.as_str())
}

fn in_place_candidate(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> ManagedCheckoutId {
    let record: Value = serde_json::from_slice(
        &fs::read(records(fixture, job.job_id()).join("inplace-root.json")).unwrap(),
    )
    .unwrap();
    let root_id = record["root_id"].as_str().unwrap();
    ManagedCheckoutId::parse(&format!(
        "inplace-{}",
        root_id.strip_prefix("loc-").unwrap()
    ))
    .unwrap()
}

#[tokio::test]
async fn in_place_read_only_job_runs() {
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-read-only-run", &provider.endpoint).await;
    let job = started(&fixture, "in-place-read-only-run", vec![]).await;
    run(&job).await;
    let status = fixture.application.status(job.job_id()).await.unwrap();
    assert_eq!(status["state"], "completed", "{status}");
    assert_eq!(status["checkouts"], json!([]), "{status}");
    assert_eq!(
        status["outcome"]["state"], "completed",
        "the run reports its outcome: {status}"
    );
    assert!(!fixture.repository.join(".git").exists());
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}

#[tokio::test]
async fn in_place_read_only_leaves_folder_untouched() {
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-read-only-write", &provider.endpoint).await;
    let job = started(&fixture, "in-place-read-only-write", vec![]).await;
    run(&job).await;
    // A job with no `--write` path is read-only (F4): no checkout exists, no
    // write tool is composed over the folder, and the folder itself is
    // byte-identical after the run.
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
    let entries = fs::read_dir(&fixture.repository)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["tracked.txt".to_string()]);
    let checkouts = fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 32)
        .await
        .unwrap();
    assert!(checkouts.items.is_empty());
    // Review reads the folder itself; with no change, it lists none.
    let candidate = in_place_candidate(&fixture, &job);
    let review = job
        .review_existing(&candidate)
        .await
        .expect("an in-place folder is reviewable");
    assert_eq!(review.preview()["mode"], "in_place");
    assert_eq!(review.preview()["changes"], json!([]));
    // Apply acknowledges that review and writes no file in the folder.
    let applied = dispatch_coding(
        &fixture.application,
        CodingCommand::Apply {
            id: job.job_id().as_str().into(),
            candidate: candidate.as_str().into(),
            source_sha256: review.source_sha256().as_str().into(),
            approve: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(applied["mode"], "in_place", "{applied}");
    assert_eq!(applied["already_applied"], json!(false), "{applied}");
    assert_eq!(
        fs::read(fixture.repository.join("tracked.txt")).unwrap(),
        b"original\n"
    );
}

#[tokio::test]
async fn in_place_scratch_cleanup() {
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-scratch-cleanup", &provider.endpoint).await;
    fs::create_dir_all(fixture.repository.join(".saya/inplace/home")).unwrap();
    fs::write(fixture.repository.join(".saya/inplace/home/known"), b"x").unwrap();
    let job = started(&fixture, "in-place-scratch-cleanup", vec![]).await;
    run(&job).await;
    assert!(
        !fixture.repository.join(".saya/inplace").exists(),
        "the known scratch layout is removed at job end"
    );
    assert!(
        !fixture.repository.join(".saya").exists(),
        "the empty scratch parent is removed too"
    );

    // A `.saya` holding a user file is left, even after the scratch goes.
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-scratch-user-saya", &provider.endpoint).await;
    fs::create_dir_all(fixture.repository.join(".saya/inplace/home")).unwrap();
    fs::write(fixture.repository.join(".saya/inplace/home/known"), b"x").unwrap();
    fs::write(fixture.repository.join(".saya/config.toml"), b"user\n").unwrap();
    let job = started(&fixture, "in-place-scratch-user-saya", vec![]).await;
    run(&job).await;
    assert!(
        fixture.repository.join(".saya/config.toml").exists(),
        "a user file in .saya is never removed"
    );
    assert!(
        !fixture.repository.join(".saya/inplace").exists(),
        "the known scratch layout is still removed"
    );

    // A foreign entry leaves the whole tree untouched: the cleanup never
    // guesses at a user's `.saya/inplace`.
    let provider = Provider::parity();
    provider.release();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-scratch-foreign", &provider.endpoint).await;
    fs::create_dir_all(fixture.repository.join(".saya/inplace")).unwrap();
    fs::write(fixture.repository.join(".saya/inplace/foreign"), b"x").unwrap();
    let job = started(&fixture, "in-place-scratch-foreign", vec![]).await;
    run(&job).await;
    assert!(
        fixture.repository.join(".saya/inplace/foreign").exists(),
        "a foreign scratch entry is never removed"
    );
}

#[tokio::test]
async fn in_place_before_images() {
    use saya_harness::coding::local::before_image::{BeforeImageStore, BeforeState};

    let provider = Provider::before_images();
    let fixture =
        Fixture::in_place_with_endpoint("in-place-before-images", &provider.endpoint).await;
    fs::write(fixture.repository.join("a.txt"), b"alpha\n").unwrap();
    fs::write(fixture.repository.join("old.txt"), b"old bytes\n").unwrap();
    // `delete` is opt-in through the job's tool list; name the four tools
    // this script uses so the write surface advertises it.
    let request = CodingJobRequest::new(
        JobId::parse("in-place-before-images").unwrap(),
        "edit, create and delete in the plain folder".into(),
        vec![".".into()],
    )
    .unwrap()
    .with_tools_allow(Some(vec![
        "read".into(),
        "edit".into(),
        "write".into(),
        "delete".into(),
    ]))
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    run(&job).await;

    let store = BeforeImageStore::open(&records(&fixture, job.job_id()))
        .expect("the job's before-image store opens");
    let entries = store.entries().expect("entries read");
    let state = |path: &str| {
        entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("{path} is recorded: {entries:?}"))
    };
    assert_eq!(
        store.read_image(state("a.txt")).expect("image reads"),
        Some(b"alpha\n".to_vec()),
        "the first edit's bytes are kept"
    );
    assert_eq!(state("new.txt").state, BeforeState::Absent);
    assert_eq!(
        store.read_image(state("old.txt")).expect("image reads"),
        Some(b"old bytes\n".to_vec()),
        "the deleted file's bytes are kept"
    );
    assert_eq!(
        fs::read(fixture.repository.join("a.txt")).unwrap(),
        b"gamma\n"
    );
    assert_eq!(
        fs::read(fixture.repository.join("new.txt")).unwrap(),
        b"fresh\n"
    );
    assert!(!fixture.repository.join("old.txt").exists());
}

#[tokio::test]
async fn in_place_second_job_same_root_refused() {
    let provider = Provider::parity();
    provider.release();
    let fixture = Fixture::in_place_with_endpoint("in-place-lease", &provider.endpoint).await;
    let first = started(&fixture, "in-place-lease-one", vec![]).await;

    let request = CodingJobRequest::new(
        JobId::parse("in-place-lease-two").unwrap(),
        "second job over the same folder".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let refusal = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .err()
        .expect("a second live job over the same in-place root is refused");
    assert!(
        matches!(refusal, CodingError::InPlaceRootBusy),
        "{refusal:?}"
    );
    drop(first);
}

#[tokio::test]
async fn in_place_write_requires_scope() {
    let fixture = Fixture::in_place("in-place-write-no-scope").await;
    let request = CodingJobRequest::new(
        JobId::parse("in-place-write-no-scope").unwrap(),
        "edit the folder".into(),
        vec![],
    )
    .unwrap()
    .with_tools_allow(Some(vec!["write".into()]))
    .unwrap();
    let proposal = fixture.application.propose(&request).await.unwrap();
    let approval = CodingAuthorization::scoped(&proposal).unwrap();
    let refusal = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .err()
        .expect("a write tool with no --write path is refused");
    assert!(matches!(refusal, CodingError::InPlaceNeedsWriteScope));
}
