//! In-place review, finalize, and the facts every surface states about them:
//! the folder is the checkout, and `apply` acknowledges without writing.

use super::support::Fixture;
use saya_agent::CancellationToken;
use saya_cli::{
    CodingAuthorization, CodingCommand, CodingError, CodingJobRequest, NativeCodingJob,
    dispatch_coding,
};
use saya_harness::coding::local::before_image::BeforeImageStore;
use saya_types::{JobId, ManagedCheckoutId, WorkspacePath};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;

async fn create(fixture: &Fixture, id: &str) -> std::sync::Arc<NativeCodingJob> {
    let request =
        CodingJobRequest::new(JobId::parse(id).unwrap(), "inspect only".into(), vec![]).unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

pub(super) async fn create_write(
    fixture: &Fixture,
    id: &str,
    paths: &[&str],
) -> std::sync::Arc<NativeCodingJob> {
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

pub(super) fn records(fixture: &Fixture, job: &JobId) -> std::path::PathBuf {
    fixture.root.join("coding").join(job.as_str())
}

/// Records the first bytes the job's tool would have seen at `path`, exactly
/// as the edit-time recorder does before an edit. `None` states the path was
/// absent at first touch.
pub(super) fn touch(fixture: &Fixture, job: &JobId, path: &str, before: Option<&[u8]>) {
    BeforeImageStore::open(&records(fixture, job))
        .unwrap()
        .record_first_touch(
            &WorkspacePath::try_from(path.to_owned()).unwrap(),
            before,
            false,
            1,
        )
        .unwrap();
}

pub(super) fn in_place_candidate(fixture: &Fixture, job: &NativeCodingJob) -> ManagedCheckoutId {
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

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
async fn in_place_review_lists_changes() {
    let fixture = Fixture::in_place("in-place-review-changes").await;
    let job = create_write(
        &fixture,
        "job-in-place-review-changes",
        &["tracked.txt", "report.md"],
    )
    .await;
    touch(&fixture, job.job_id(), "tracked.txt", Some(b"original\n"));
    touch(&fixture, job.job_id(), "report.md", None);
    fs::write(fixture.repository.join("tracked.txt"), b"changed\n").unwrap();
    fs::write(fixture.repository.join("report.md"), b"new\n").unwrap();
    let candidate = in_place_candidate(&fixture, &job);
    let review = job
        .review_existing(&candidate)
        .await
        .expect("an in-place folder is reviewable");
    let preview = review.preview();
    assert_eq!(preview["mode"], "in_place", "{preview}");
    assert_eq!(
        preview["baseline"]["kind"], "edit_time",
        "the baseline is an edit-time marker: {preview}"
    );
    assert_eq!(preview["apply_requires_fresh_approval"], json!(false));
    assert_eq!(preview["undo"]["baseline_restore"], json!(true));
    let changes = preview["changes"].as_array().unwrap();
    let paths = changes
        .iter()
        .map(|change| change["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["report.md", "tracked.txt"], "{preview}");
    assert_eq!(changes[0]["kind"], "Added");
    assert_eq!(changes[1]["kind"], "Modified");
    assert!(preview["diff"]["files"].is_array(), "{preview}");
    assert_eq!(review.source_sha256().as_str().len(), 64);
    let record = fs::read_to_string(
        records(&fixture, job.job_id()).join(format!("review-{}.jsonl", candidate.as_str())),
    )
    .unwrap();
    assert!(record.contains("\"mode\":\"in_place\""), "{record}");
}

#[tokio::test]
async fn in_place_review_stale_root_refused() {
    let fixture = Fixture::in_place("in-place-review-stale-root").await;
    let job = create_write(&fixture, "job-in-place-review-stale-root", &["tracked.txt"]).await;
    let candidate = in_place_candidate(&fixture, &job);
    let moved = fixture.root.join("repo-moved");
    fs::rename(&fixture.repository, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &fixture.repository).unwrap();
    let error = job
        .review_existing(&candidate)
        .await
        .err()
        .expect("a root that is no longer a plain folder is refused");
    assert!(
        !matches!(error, CodingError::InPlaceRequiresGit { .. }),
        "review reaches the root check: {error}"
    );
    assert!(matches!(error, CodingError::Owner(_)), "{error}");
}

#[tokio::test]
async fn foreign_change_detected_and_marked() {
    let fixture = Fixture::in_place("in-place-review-foreign").await;
    fs::write(fixture.repository.join("removed.txt"), b"remove me\n").unwrap();
    let job = create_write(
        &fixture,
        "job-in-place-review-foreign",
        &["tracked.txt", "removed.txt"],
    )
    .await;
    let candidate = in_place_candidate(&fixture, &job);
    touch(&fixture, job.job_id(), "tracked.txt", Some(b"original\n"));
    touch(&fixture, job.job_id(), "removed.txt", Some(b"remove me\n"));
    touch(&fixture, job.job_id(), "foreign.txt", None);
    fs::write(fixture.repository.join("tracked.txt"), b"changed\n").unwrap();
    fs::remove_file(fixture.repository.join("removed.txt")).unwrap();
    let journal =
        records(&fixture, job.job_id()).join(format!("known-commits-{}.jsonl", candidate.as_str()));
    let record = |operation: &str, path: &str, bytes: &[u8]| {
        json!({
            "schema_version": 1,
            "operation": operation,
            "path": path,
            "committed_sha256": sha256_hex(bytes),
            "size": bytes.len(),
            "turn": 1,
        })
    };
    fs::write(
        &journal,
        format!(
            "{}\n{}\n",
            record("write", "tracked.txt", b"changed\n"),
            record("delete", "removed.txt", b"remove me\n"),
        ),
    )
    .unwrap();
    fs::write(fixture.repository.join("foreign.txt"), b"user\n").unwrap();
    let review = job.review_existing(&candidate).await.unwrap();
    assert_eq!(
        review.preview()["foreign_changes"],
        json!(["foreign.txt"]),
        "the journaled write and the job's own removal are not foreign: {}",
        review.preview()
    );
}

#[tokio::test]
async fn in_place_proposal_exact_keys() {
    let fixture = Fixture::in_place("in-place-proposal").await;
    let request = CodingJobRequest::new(
        JobId::parse("job-in-place-proposal").unwrap(),
        "edit the folder".into(),
        vec!["report.md".into()],
    )
    .unwrap();
    let proposal = fixture.application.propose(&request).await.unwrap();
    let value = proposal.request();
    assert_eq!(value["mode"], "in_place", "{value}");
    assert_eq!(value["root"]["git"], json!(false), "{value}");
    assert_eq!(
        value["root"]["path"],
        json!(fixture.repository.canonicalize().unwrap().to_str().unwrap())
    );
    assert_eq!(value["write_scope"], json!(["report.md"]), "{value}");
    assert_eq!(value["baseline"]["kind"], "edit_time", "{value}");
    assert!(
        value["baseline"].get("files").is_none(),
        "an edit-time baseline holds no file inventory: {value}"
    );
    assert_eq!(value["scratch"], ".saya/inplace", "{value}");
    assert_eq!(value["no_git_history"], json!(true), "{value}");
    assert_eq!(value["undo"]["checkpoints"], json!(32), "{value}");
    assert_eq!(value["undo"]["baseline_restore"], json!(true), "{value}");
    assert_eq!(value["undo"]["commands"], json!(false), "{value}");
    assert!(
        value["shell_reach"]
            .as_str()
            .is_some_and(|reach| reach.contains("whole folder")),
        "{value}"
    );
}

#[test]
fn in_place_approval_text() {
    let in_place = json!({"mode": "in_place", "root": {"path": "/tmp/folder"}});
    let definition = CodingAuthorization::definition_for(&in_place);
    assert_eq!(definition.name, "coding_job");
    let description = definition.description.to_ascii_lowercase();
    assert!(
        description.contains("in place"),
        "{}",
        definition.description
    );
    assert!(description.contains("live"), "{}", definition.description);
    assert!(
        description.contains("no git history"),
        "{}",
        definition.description
    );
    assert_eq!(
        CodingAuthorization::definition_for(&json!({"mode": "git"})).description,
        CodingAuthorization::definition().description,
        "a Git proposal keeps its exact wording"
    );
}

#[tokio::test]
async fn export_in_place_facts() {
    let fixture = Fixture::in_place("in-place-export").await;
    let job = create(&fixture, "job-in-place-export").await;
    let record = fixture
        .store
        .get_coding_job(job.job_id())
        .await
        .unwrap()
        .unwrap();
    let export = dispatch_coding(
        &fixture.application,
        CodingCommand::Export {
            id: job.job_id().as_str().into(),
            after: None,
            transport_after: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(export["mode"], "in_place", "{export}");
    assert_eq!(
        export["root"]["path"],
        json!(fixture.repository.canonicalize().unwrap().to_str().unwrap())
    );
    assert_eq!(export["root"]["git"], json!(false), "{export}");
    assert_eq!(
        export["baseline"]["snapshot_id"],
        json!(record.snapshot_id.as_str()),
        "{export}"
    );
    assert_eq!(export["baseline"]["kind"], "edit_time", "{export}");
    assert_eq!(export["recorded"]["files"], 0, "{export}");
    assert_eq!(export["recorded"]["bytes"], 0, "{export}");
    assert_eq!(export["recorded"]["overflowed"], false, "{export}");
}

#[tokio::test]
async fn in_place_doctor_reports_no_worktree() {
    let fixture = Fixture::in_place("in-place-doctor").await;
    let _job = create(&fixture, "job-in-place-doctor").await;
    let report = fixture.application.doctor(false).await.unwrap();
    assert_eq!(report["mode"], "in_place", "{report}");
    assert!(report["worktree"].is_null(), "{report}");
    assert_eq!(report["root"]["git"], json!(false), "{report}");
    assert_eq!(
        report["root"]["path"],
        json!(fixture.repository.canonicalize().unwrap().to_str().unwrap())
    );
    assert_eq!(report["scratch"], json!(".saya/inplace"), "{report}");
}

#[tokio::test]
async fn in_place_worktrees_empty() {
    let fixture = Fixture::in_place("in-place-worktrees").await;
    let job = create(&fixture, "job-in-place-worktrees").await;
    let listed = fixture.application.worktrees(job.job_id()).await.unwrap();
    assert_eq!(listed["mode"], "in_place", "{listed}");
    assert_eq!(listed["checkouts"], json!([]), "{listed}");
    assert_eq!(listed["root"]["git"], json!(false), "{listed}");
}
