//! The five resume decisions, end to end through the line front: one row per
//! row of `chat::link::decide`, each driven by `saya --resume <id>` over a
//! record the binary itself saved.

use crate::harness::{Round, script};
use crate::resume_paths_fixture::*;
use crate::support::cleanup;

/// Attach: the linked run is still open, so the next line runs on it — the
/// record still names the job it named, and the run continues as it stands.
#[test]
fn an_open_run_is_attached_and_the_next_line_runs_on_it() {
    let env = fixture("q01-resume-attach");
    let scripted = script(vec![
        Box::new(|_| Round::Text(FIRST_ANSWER)),
        Box::new(|_| Round::Text("attached answer")),
    ]);
    let (id, job) = first_process(&env, &scripted);

    let resumed = resume(&env, &scripted, &id, RESUME_LINE);
    assert!(
        resumed.status.success(),
        "the resumed process exited cleanly\nstdout:\n{}\nstderr:\n{}",
        resumed.stdout,
        resumed.stderr
    );
    assert_eq!(
        linked_job(&crate::support::saved_record(&env, &id)),
        job,
        "the next line ran on the run the record named"
    );
    assert!(
        !carried_answer(&resumed.request),
        "an attached run continues as it stands, with no seeded history: {}",
        resumed.request
    );

    cleanup(&env);
}

/// PrepareNewEpoch: the finished run gives way to the same job's next epoch,
/// seeded with the kept tail.
#[test]
fn a_finished_run_prepares_the_next_epoch_seeded_with_the_kept_tail() {
    let env = fixture("q01-resume-new-epoch");
    let scripted = script(vec![
        Box::new(|_| Round::Text(FIRST_ANSWER)),
        Box::new(|_| Round::Text("job answer")),
        Box::new(|_| Round::Text("second answer")),
    ]);
    let (id, _session_job) = first_process(&env, &scripted);
    // The run the record links is over: the binary started it and finished it.
    let job = finished_job(&env, &scripted, "finished-run-051");
    edit_record(&env, &id, |record| {
        record["run"]["job"] = job.as_str().into();
        record["run"]["epoch"] = 1.into();
    });

    let resumed = resume(&env, &scripted, &id, RESUME_LINE);
    assert!(
        resumed.status.success(),
        "the resumed process exited cleanly\nstdout:\n{}\nstderr:\n{}",
        resumed.stdout,
        resumed.stderr
    );
    let record = crate::support::saved_record(&env, &id);
    assert_eq!(
        linked_job(&record),
        job,
        "the next run is on the job the record named, not a new one"
    );
    assert_eq!(
        record["run"]["epoch"].as_u64(),
        Some(2),
        "and it is that job's next epoch: {record}"
    );
    assert!(
        carried_answer(&resumed.request),
        "the request carries the kept tail: {}",
        resumed.request
    );

    cleanup(&env);
}

/// ReadOnly, no link: a 0.5.0 record holds no run link, so the record opens
/// read-only and the next line prepares a run of its own.
#[test]
fn a_record_without_a_link_opens_read_only_and_the_next_line_prepares_a_run() {
    let env = fixture("q01-resume-no-link");
    let scripted = script(vec![
        Box::new(|_| Round::Text(FIRST_ANSWER)),
        Box::new(|_| Round::Text("fresh answer")),
    ]);
    let (id, _job) = first_process(&env, &scripted);
    edit_record(&env, &id, |record| {
        record.as_object_mut().unwrap().remove("run");
    });

    let resumed = resume(&env, &scripted, &id, RESUME_LINE);
    assert!(
        resumed.status.success(),
        "the resumed process exited cleanly\nstdout:\n{}\nstderr:\n{}",
        resumed.stdout,
        resumed.stderr
    );
    assert!(
        resumed.stdout.contains("carries no run link"),
        "the record says it opens read-only: {}",
        resumed.stdout
    );
    assert!(
        resumed.request != serde_json::Value::Null,
        "the next line prepared a run and answered on it: {}",
        resumed.stdout
    );

    cleanup(&env);
}

/// ReadOnly, missing folder: the job the link names keeps no records here, so
/// the record opens read-only and the next line prepares a new job.
#[test]
fn a_record_whose_run_dir_is_missing_opens_read_only_and_prepares_a_run() {
    let env = fixture("q01-resume-missing-dir");
    let scripted = script(vec![
        Box::new(|_| Round::Text(FIRST_ANSWER)),
        Box::new(|_| Round::Text("fresh answer")),
    ]);
    let (id, _job) = first_process(&env, &scripted);
    edit_record(&env, &id, |record| {
        record["run"]["job"] = "gone-run-051".into()
    });

    let resumed = resume(&env, &scripted, &id, RESUME_LINE);
    assert!(
        resumed.status.success(),
        "the resumed process exited cleanly\nstdout:\n{}\nstderr:\n{}",
        resumed.stdout,
        resumed.stderr
    );
    assert!(
        resumed.stdout.contains("folder is gone") && resumed.stdout.contains("read-only"),
        "the record says why it opens read-only: {}",
        resumed.stdout
    );
    assert!(
        resumed.request != serde_json::Value::Null,
        "the next line prepared a run and answered on it: {}",
        resumed.stdout
    );

    cleanup(&env);
}

/// Refused: a job in a managed checkout takes one turn per run, so the session
/// says so once and the process leaves with exit 2.
#[test]
fn a_managed_record_refuses_with_one_sentence_and_exits_two() {
    let env = fixture("q01-resume-managed");
    let scripted = script(vec![
        Box::new(|_| Round::Text(FIRST_ANSWER)),
        Box::new(|_| Round::Text("never asked")),
    ]);
    let (id, _job) = first_process(&env, &scripted);
    edit_record(&env, &id, |record| {
        record["run"]["job"] = "managed-run-051".into()
    });
    // A managed job's records state no in-place root: the folder is a checkout
    // the job does not write.
    std::fs::create_dir_all(job_dir(&env, "managed-run-051")).unwrap();
    let asked_before = scripted.bodies.lock().unwrap().len();

    let resumed = resume_without_a_turn(&env, &scripted, &id);
    assert_eq!(
        resumed.status.code(),
        Some(2),
        "a refused resume exits 2\nstdout:\n{}\nstderr:\n{}",
        resumed.stdout,
        resumed.stderr
    );
    let said = format!("{}{}", resumed.stdout, resumed.stderr);
    assert_eq!(
        said.matches("managed checkout").count(),
        1,
        "one sentence, said once: {said}"
    );
    assert_eq!(
        scripted.bodies.lock().unwrap().len(),
        asked_before,
        "nothing was asked of the provider: {said}"
    );

    cleanup(&env);
}
