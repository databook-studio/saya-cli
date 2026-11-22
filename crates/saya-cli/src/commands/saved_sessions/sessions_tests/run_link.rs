// What deleting a saved session does with the job its record links: the
// session's own finished run goes with the record, a run that is still open
// refuses the whole delete, and a job the session did not open is never
// touched.

use crate::commands::coding::{CodingJobRequest, JobOrigin};
use saya_store::RunLink;
use saya_types::JobId;
use std::{fs, path::Path};

fn job_id(id: &str) -> JobId {
    JobId::parse(id).expect("the fixture's job id parses")
}

/// The request a job of `origin` was created with, written where the job's own
/// records keep it.
fn write_request(job: &Path, id: &str, origin: JobOrigin) {
    let request = CodingJobRequest::new(job_id(id), format!("session {id}"), Vec::new())
        .expect("the fixture's request is valid")
        .with_origin(origin)
        .expect("the fixture's origin is valid");
    fs::write(
        job.join("request.json"),
        serde_json::to_vec(&request).expect("the request serializes"),
    )
    .expect("the job's records are writable");
}

/// A job's records directory under `jobs`, with its request and — when the job
/// recorded one — its terminal outcome block.
fn job_records(jobs: &Path, id: &str, origin: JobOrigin, state: Option<&str>) -> std::path::PathBuf {
    let job = jobs.join(id);
    fs::create_dir_all(&job).expect("the job's records are creatable");
    write_request(&job, id, origin);
    if let Some(state) = state {
        fs::write(job.join("outcome.json"), format!("{{\"state\":\"{state}\"}}"))
            .expect("the outcome block is writable");
    }
    job
}

/// A saved session whose record links `job`: the link a session writes once its
/// run is prepared.
fn linked_record(sessions: &Path, id: &str, job: &str) {
    let record = RedactedSession {
        id: id.to_owned(),
        run: Some(RunLink {
            job: job_id(job),
            epoch: 1,
        }),
        ..RedactedSession::default()
    };
    fs::write(
        sessions.join(format!("{id}.json")),
        serde_json::to_vec(&record).expect("the record serializes"),
    )
    .expect("the record is writable");
}

#[test]
fn a_finished_session_run_is_deleted_with_its_record() {
    let sessions = scratch("link-finished");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    linked_record(&sessions, "sess-one", "job-one");
    let job = job_records(&jobs, "job-one", JobOrigin::Session, Some("completed"));
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, &jobs, "sess-one", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(
        !sessions.join("sess-one.json").exists(),
        "the record is gone"
    );
    assert!(!job.exists(), "the session's own finished run is gone");
    let (code, text) = render_delete(&outcome, "sess-one");
    assert_eq!(code, 0, "{text}");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn a_live_run_refuses_the_delete_and_keeps_both() {
    let sessions = scratch("link-live");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    linked_record(&sessions, "sess-live", "job-live");
    let job = job_records(&jobs, "job-live", JobOrigin::Session, None);
    // The in-place root the job works, and the lease a live run holds on it.
    fs::write(
        job.join("inplace-root.json"),
        format!(
            "{{\"version\":2,\"root_id\":\"loc-root-live\",\"root_path\":\"/tmp/site\",\
             \"snapshot_id\":\"snap-live\",\"plan_sha256\":\"{}\",\"git\":false}}",
            "a".repeat(64)
        ),
    )
    .expect("the root record is writable");
    let _lease = hold_lease(&jobs, "loc-root-live");
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, &jobs, "sess-live", None);
    assert_eq!(
        outcome,
        DeleteOutcome::LiveRun {
            job: "job-live".to_owned()
        },
        "a held lease refuses the delete"
    );
    assert!(sessions.join("sess-live.json").exists(), "the record is kept");
    assert!(job.exists(), "the job is kept");
    assert!(
        !base.join("lifecycle").exists(),
        "no tombstone was written"
    );
    let (code, text) = render_delete(&outcome, "sess-live");
    assert_eq!(code, EXIT_SESSIONS_ERROR);
    assert!(text.contains("job-live"), "the refusal names the job: {text}");
    assert!(
        text.contains(RUN_STILL_OPEN),
        "the refusal is the one sentence: {text}"
    );
    let _ = fs::remove_dir_all(base);
}

#[test]
fn a_job_the_command_line_opened_is_kept_when_the_session_goes() {
    let sessions = scratch("link-cli");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    linked_record(&sessions, "sess-cli", "job-cli");
    let job = job_records(&jobs, "job-cli", JobOrigin::Cli, Some("completed"));
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, &jobs, "sess-cli", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(
        !sessions.join("sess-cli.json").exists(),
        "the record still goes"
    );
    assert!(job.exists(), "a job the CLI opened is never touched");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn a_job_that_has_not_finished_is_kept() {
    let sessions = scratch("link-unfinished");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    linked_record(&sessions, "sess-paused", "job-paused");
    let job = job_records(&jobs, "job-paused", JobOrigin::Session, None);
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, &jobs, "sess-paused", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(
        !sessions.join("sess-paused.json").exists(),
        "the record goes"
    );
    assert!(job.exists(), "a run that has not finished is kept");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn a_record_without_a_link_deletes_as_it_always_did() {
    let sessions = scratch("link-none");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    fs::write(sessions.join("plain.json"), "{\"id\":\"plain\"}").unwrap();
    let unrelated = job_records(&jobs, "job-other", JobOrigin::Session, Some("completed"));
    let store = FsSessionStore::new(&sessions);
    let outcome = delete_saved_session(&store, &sessions, &jobs, "plain", None);
    assert_eq!(outcome, DeleteOutcome::Deleted, "{outcome:?}");
    assert!(!sessions.join("plain.json").exists(), "the record is gone");
    assert!(
        unrelated.exists(),
        "a record with no link names no job to remove"
    );
    let _ = fs::remove_dir_all(base);
}

/// Holds a folder's lease the way a live run does: one open descriptor on the
/// same lock file, taken with the same non-blocking `flock` a run takes.
#[cfg(unix)]
fn hold_lease(jobs: &Path, root_id: &str) -> fs::File {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let directory = jobs.join("inplace-leases");
    fs::create_dir_all(&directory).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(directory.join(format!("{root_id}.lock")))
        .unwrap();
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(locked, 0, "the test takes the lease");
    file
}

/// A host without `flock` never reads a lease as held, so the refusal case has
/// nothing to hold.
#[cfg(not(unix))]
fn hold_lease(_jobs: &Path, _root_id: &str) -> fs::File {
    panic!("this case needs a host that can hold a lease")
}
