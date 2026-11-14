//! Tests for the approved coding job request. Direct coordination is the only
//! behaviour, so a `request.json` written by an older binary carries no mode
//! and still parses and resumes; `deny_unknown_fields` still refuses anything
//! else; and the start proposal names no mode.

use super::CodingJobRequest;
use super::request_fixture::{JOB, fixture};
use std::fs;

#[cfg(unix)]
#[tokio::test]
async fn an_older_request_json_without_a_mode_still_parses_and_resumes() {
    let fixture = fixture("legacy-resume").await;
    // The bytes an older binary wrote for this exact job: no coordinator_mode.
    let bytes = br#"{"job_id":"request-mode-legacy-resume","task":"resume a request written by an older binary","write_paths":["tracked.txt"],"programs":[],"database_profiles":[],"database_identities":{}}"#;
    let request: CodingJobRequest = serde_json::from_slice(bytes).unwrap();
    request.validate().unwrap();
    assert_eq!(request.job_id().as_str(), JOB);
    // Resuming replays the request from disk, exactly as `resume` does.
    fs::write(fixture.job.records.join("request.json"), bytes).unwrap();
    let resumed = fixture
        .application
        .continuation_request(fixture.job.job_id())
        .unwrap();
    assert_eq!(resumed.job_id().as_str(), JOB);
    assert_eq!(
        resumed.task(),
        "resume a request written by an older binary"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_request_json_with_an_unknown_field_is_refused() {
    let bytes = br#"{"job_id":"request-mode-unknown","task":"an unknown field","write_paths":[],"programs":[],"surprise":true}"#;
    assert!(
        serde_json::from_slice::<CodingJobRequest>(bytes).is_err(),
        "deny_unknown_fields must refuse a field the request does not have"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_start_proposal_does_not_name_a_coordinator_mode() {
    let fixture = fixture("proposal-no-mode").await;
    let proposal = fixture
        .application
        .propose(&fixture.job.request)
        .await
        .unwrap();
    assert!(
        proposal.request().get("coordinator_mode").is_none(),
        "the approval proposal must not carry a coordinator mode: {}",
        proposal.request()
    );
}
