//! What a readiness view says about the walk behind its source generation: the
//! one refusal that is reported — a checkout that kept losing entries under the
//! walk — and every refusal that keeps the handling it had.
//!
//! The report is the job's own observation path, not a log line nobody
//! collects: one record carrying the fixed phase and the fixed class, and
//! nothing about the path that left the tree.

use super::super::direct_root_fixture::{RootFixture, fixture, root_path};
use super::{CodingError, NativeCodingJob, reported, vanished_class};
use crate::commands::coding::prepare::SourceGeneration;
use saya_harness::coding::integration::{CandidateError, DeltaError, EvidenceError};
use saya_types::{ManagedCheckoutId, Sha256Digest};
use serde_json::Value;

/// The phase a gave-up walk records. Written out rather than imported: the
/// record's phase is fixed vocabulary, so a rename must show up in this test.
const GAVE_UP: &str = "readiness_walk_gave_up";

/// The refusal the readiness walk produces when it gives up: it lost an entry
/// between the listing and the read on each of its three attempts.
fn walked(error: DeltaError) -> CodingError {
    CodingError::owner(EvidenceError::Unverifiable(CandidateError::Walk(error)))
}

/// One job of its own, so a record one test's walk wrote is never read by
/// another's.
async fn job(case: &str) -> RootFixture {
    fixture(case, vec!["tracked.txt".into()]).await
}

/// The JSON lines of one of the job's private diagnostic streams.
fn observations(job: &NativeCodingJob) -> Vec<Value> {
    let Ok(bytes) = std::fs::read_to_string(job.records.join("observations.jsonl")) else {
        return Vec::new();
    };
    bytes
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The job's records that state a gave-up readiness walk.
fn gave_up(job: &NativeCodingJob) -> Vec<Value> {
    observations(job)
        .into_iter()
        .filter(|record| record["phase"] == GAVE_UP)
        .collect()
}

#[tokio::test]
async fn a_walk_that_gave_up_on_a_vanished_entry_is_reported_once_and_stays_unknown() {
    let fixture = job("gave-up").await;
    let error = walked(DeltaError::Vanished);
    assert_eq!(
        vanished_class(&error),
        Some("vanished"),
        "the class behind the record names what happened, and nothing else"
    );
    assert!(
        reported(&fixture.job, Err(error)).await.is_none(),
        "the readiness view stays unknown, as it did when every refusal was swallowed"
    );
    let recorded = gave_up(&fixture.job);
    assert_eq!(
        recorded.len(),
        1,
        "one gave-up walk is exactly one observation: {recorded:?}"
    );
    assert_eq!(recorded[0]["phase"], "readiness_walk_gave_up");
    assert_eq!(recorded[0]["class"], "vanished");
    let text = recorded[0].to_string();
    assert!(
        !text.contains(
            root_path(&fixture.job)
                .to_str()
                .expect("the checkout path is UTF-8")
        ),
        "the record carries no path: {text}"
    );
}

#[tokio::test]
async fn a_settled_walk_records_none() {
    let fixture = job("settled").await;
    // W is a real checkout that holds still, so its walk finishes and produces
    // the generation the readiness view reports.
    let settled = reported(&fixture.job, fixture.job.source_generation_direct().await).await;
    assert!(
        settled.is_some(),
        "a walk that settles reports the generation it produced"
    );
    assert!(
        gave_up(&fixture.job).is_empty(),
        "a settled walk records no observation: {:?}",
        observations(&fixture.job)
    );
}

#[tokio::test]
async fn every_other_refusal_keeps_todays_handling() {
    let fixture = job("other-refusals").await;
    for class in [
        DeltaError::Io,
        DeltaError::Cancelled,
        DeltaError::DeadlineExceeded,
        DeltaError::TooManyFiles,
        DeltaError::UnsafePath,
    ] {
        let error = walked(class);
        assert_eq!(
            vanished_class(&error),
            None,
            "only a vanished entry is reported: {class:?}"
        );
        assert!(
            reported(&fixture.job, Err(error)).await.is_none(),
            "a refusal that is not a vanished entry still yields no generation: {class:?}"
        );
    }
    assert!(
        reported(&fixture.job, Err(CodingError::InvalidRequest))
            .await
            .is_none(),
        "an error with no walk behind it yields no generation either"
    );
    assert!(
        gave_up(&fixture.job).is_empty(),
        "no other refusal records a gave-up observation"
    );
}

#[tokio::test]
async fn a_generation_that_the_walk_produced_is_reported_unchanged() {
    let fixture = job("produced").await;
    let generation = SourceGeneration {
        digest: Sha256Digest::parse(&"a".repeat(64)).expect("a sixty-four character digest"),
        candidate: ManagedCheckoutId::parse("candidate-a").expect("a candidate id"),
    };
    let reported = reported(&fixture.job, Ok(generation))
        .await
        .expect("a produced generation is kept");
    assert_eq!(reported.candidate.as_str(), "candidate-a");
    assert!(gave_up(&fixture.job).is_empty());
}
