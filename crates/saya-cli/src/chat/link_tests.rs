//! The link a session writes: the job and epoch once its run is prepared, and
//! the next epoch — or the next job — once it rolls over.

use saya_store::{RedactedSession, RunLink};
use saya_types::JobId;

use super::{after_prepare, after_rollover};

fn job(id: &str) -> JobId {
    JobId::parse(id).expect("the fixture's job id parses")
}

/// A saved record from before the session kept a link.
fn unlinked() -> RedactedSession {
    RedactedSession {
        id: "chat-051".to_owned(),
        model: "current-model".to_owned(),
        ..RedactedSession::default()
    }
}

#[test]
fn the_link_is_written_after_the_run_is_prepared() {
    let mut record = unlinked();
    after_prepare(&mut record, &job("job-051"), 1);
    assert_eq!(
        record.run,
        Some(RunLink {
            job: job("job-051"),
            epoch: 1
        }),
        "the prepared run is linked into the record it will be saved with"
    );
    let text = serde_json::to_string(&record).expect("the record serializes");
    assert!(
        text.contains("\"run\":{\"job\":\"job-051\",\"epoch\":1}"),
        "the saved record names the job and the epoch: {text}"
    );
}

#[test]
fn a_rollover_replaces_the_link_with_the_next_epoch() {
    let mut record = unlinked();
    after_prepare(&mut record, &job("job-051"), 1);
    after_rollover(&mut record, &job("job-051"), 2);
    assert_eq!(
        record.run,
        Some(RunLink {
            job: job("job-051"),
            epoch: 2
        }),
        "a rollover inside one job moves the link to the new epoch"
    );
}

#[test]
fn a_rollover_into_a_new_job_links_the_new_job_at_its_first_epoch() {
    let mut record = unlinked();
    after_prepare(&mut record, &job("job-051"), 3);
    after_rollover(&mut record, &job("job-052"), 1);
    assert_eq!(
        record.run,
        Some(RunLink {
            job: job("job-052"),
            epoch: 1
        }),
        "a rollover into a new job replaces the job as well as the epoch"
    );
}
