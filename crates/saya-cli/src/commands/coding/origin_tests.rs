//! A job's origin: where it was opened from, what a `request.json` written
//! before the key existed still reads as, and what `saya job list` does with it.
//!
//! The origin is a listing label and never authority: a session's job is shown,
//! hidden and shown again by `--all`, and its own `show` works exactly as a CLI
//! job's does.

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_types::JobId;

use super::request_fixture::{Fixture, JOB, fixture, fixture_with};
use super::{CodingApplication, CodingAuthorization, CodingJobRequest, JobOrigin, NativeCodingJob};
use crate::cli::CodingCommand;

/// The exact bytes a CLI job's `request.json` holds: the shape an older binary
/// wrote, with no origin key at all.
const CLI_REQUEST_BYTES: &[u8] = concat!(
    r#"{"job_id":"request-mode-legacy-resume","task":"resume a request written by an older binary","#,
    r#""write_paths":["tracked.txt"],"database_profiles":[],"database_identities":{}}"#
)
    .as_bytes();

const SESSION_JOB: &str = "session-origin-listed";

#[test]
fn an_older_request_json_without_an_origin_parses_as_cli_and_re_serialises_unchanged() {
    let request: CodingJobRequest = serde_json::from_slice(CLI_REQUEST_BYTES).unwrap();
    assert_eq!(
        request.origin(),
        JobOrigin::Cli,
        "a request written before the key existed is a CLI job"
    );
    assert_eq!(
        serde_json::to_vec(&request).unwrap(),
        CLI_REQUEST_BYTES,
        "a CLI job's origin is absent from the wire, not a stated default"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_job_created_from_the_cli_persists_the_bytes_it_always_did() {
    let fixture = fixture("origin-cli-bytes").await;
    let bytes = std::fs::read(fixture.job.records.join("request.json")).unwrap();
    assert_eq!(
        bytes, CLI_REQUEST_BYTES,
        "a CLI job's request is byte-identical to one written before the key existed"
    );
    let resumed = fixture
        .application
        .continuation_request(fixture.job.job_id())
        .unwrap();
    assert_eq!(resumed.origin(), JobOrigin::Cli);
}

#[cfg(unix)]
#[tokio::test]
async fn a_session_job_states_its_origin_and_resumes_with_it() {
    let fixture = fixture_with("origin-session", |request| {
        request.with_origin(JobOrigin::Session).unwrap()
    })
    .await;
    let written =
        String::from_utf8(std::fs::read(fixture.job.records.join("request.json")).unwrap())
            .unwrap();
    assert!(
        written.contains(r#""origin":"session""#),
        "the session's job states where it was opened from: {written}"
    );
    let resumed = fixture
        .application
        .continuation_request(fixture.job.job_id())
        .unwrap();
    assert_eq!(
        resumed.origin(),
        JobOrigin::Session,
        "a resume reads back the origin the job started as"
    );
}

/// Opens a second job in the fixture's own repository, so one listing holds a
/// CLI job and a session job side by side.
async fn open_job(application: &Arc<CodingApplication>, job: &str) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        format!("the {job} job"),
        Vec::new(),
    )
    .unwrap()
    .with_origin(JobOrigin::Session)
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

async fn both_jobs(case: &str) -> Fixture {
    let fixture = fixture(case).await;
    open_job(&fixture.application, SESSION_JOB).await;
    fixture
}

fn ids(listed: &serde_json::Value) -> Vec<String> {
    listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|job| job["job_id"].as_str().map(str::to_owned))
        .collect()
}

fn origins(listed: &serde_json::Value) -> Vec<(String, String)> {
    listed["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| {
            (
                job["job_id"].as_str().unwrap().to_owned(),
                job["origin"].as_str().unwrap_or("<no origin>").to_owned(),
            )
        })
        .collect()
}

#[cfg(unix)]
#[tokio::test]
async fn a_listing_hides_the_jobs_a_session_opened() {
    let listing = both_jobs("origin-list-default").await;
    let listed = super::dispatch_coding(
        &listing.application,
        CodingCommand::List {
            after: None,
            all: false,
        },
    )
    .await
    .unwrap();
    let ids = ids(&listed);
    assert!(
        ids.contains(&JOB.to_owned()),
        "the CLI's job is listed: {listed}"
    );
    assert!(
        !ids.contains(&SESSION_JOB.to_owned()),
        "a session's job is hidden by default: {listed}"
    );
    assert_eq!(
        origins(&listed),
        vec![(JOB.to_owned(), "cli".to_owned())],
        "a default listing states the origin of every job it shows: {listed}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn all_shows_both_kinds_with_their_origin() {
    let listing = both_jobs("origin-list-all").await;
    let listed = super::dispatch_coding(
        &listing.application,
        CodingCommand::List {
            after: None,
            all: true,
        },
    )
    .await
    .unwrap();
    let mut origins = origins(&listed);
    origins.sort();
    assert_eq!(
        origins,
        vec![
            (JOB.to_owned(), "cli".to_owned()),
            (SESSION_JOB.to_owned(), "session".to_owned()),
        ],
        "--all shows both kinds and names where each was opened: {listed}"
    );
}

/// The origin is a listing filter, never an authority: a session's job is shown
/// by id exactly as a CLI job's is.
#[cfg(unix)]
#[tokio::test]
async fn a_session_job_is_shown_by_id() {
    let listing = both_jobs("origin-show").await;
    let shown = super::dispatch_coding(
        &listing.application,
        CodingCommand::Show {
            id: SESSION_JOB.to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        shown["job_id"].as_str(),
        Some(SESSION_JOB),
        "a session job's own show works: {shown}"
    );
}
