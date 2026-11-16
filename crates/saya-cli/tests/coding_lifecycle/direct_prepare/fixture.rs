//! The direct-mode preparation fixture: a direct-mode job over a working
//! checkout W, plus the bounded lookups the preparation and finish tests assert
//! over (the private records and the checkouts). Shared by `direct_prepare`,
//! `direct_finish` and `direct_implicit_join`.
//!
//! Checks are removed, so a prepared receipt carries `checks: []` and
//! `candidate_ready: true`: the operator's review and apply is the check. The
//! fixture therefore configures no check profile and grants the caller's
//! programs as plain runner grants.

use crate::support::Fixture;
use saya_agent::{CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest};
use saya_types::JobId;
use std::{path::PathBuf, sync::Arc, time::Duration};

/// Opens the fixture's application over the given runner programs.
async fn fixture_with_programs(
    label: &str,
    provider: &crate::provider::Provider,
    programs: &[&str],
) -> Fixture {
    let mut fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    fixture.program_dir = crate::support::program_dir_for(programs);
    fixture.application = CodingApplication::open(
        fixture.runtime.clone(),
        fixture.store.clone(),
        &fixture.repository,
        &fixture.root.join("coding"),
        &fixture.git,
        &fixture.program_dir,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    fixture
}

/// Runs a direct-mode job to completion: a scripted provider at `mode`, the
/// approved write paths, the granted runner programs, and the run's own output
/// (the final envelope) for the callers that assert over it.
pub(crate) async fn run_direct_prepare_output(
    label: &str,
    mode: u8,
    write_paths: Vec<String>,
    programs: Vec<String>,
) -> (
    Fixture,
    Arc<saya_cli::NativeCodingJob>,
    crate::provider::Provider,
    saya_agent::AgentOutput,
) {
    let provider = crate::provider::Provider::script(false, mode);
    let names = programs.iter().map(String::as_str).collect::<Vec<_>>();
    let fixture = fixture_with_programs(label, &provider, &names).await;
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Direct-mode preparation work with helpers".into(),
        write_paths,
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(60), job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap();
    (fixture, job, provider, output)
}

pub(crate) fn records_dir(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> PathBuf {
    fixture.root.join("coding").join(job.job_id().as_str())
}

/// The durable prepared receipts in the job's private records.
pub(crate) fn prepared_receipts(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> usize {
    std::fs::read_dir(records_dir(fixture, job))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("prepared-candidate-")
                })
                .count()
        })
        .unwrap_or(0)
}

/// The lines the job's durable review records hold. A replay of an immutable
/// receipt writes no new review, so the count pins "replay reruns no review".
pub(crate) fn review_lines(fixture: &Fixture, job: &saya_cli::NativeCodingJob) -> usize {
    std::fs::read_dir(records_dir(fixture, job))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("review-"))
                .map(|entry| {
                    std::fs::read_to_string(entry.path())
                        .unwrap_or_default()
                        .lines()
                        .count()
                })
                .sum()
        })
        .unwrap_or(0)
}

/// Resumes the job under a fresh scoped approval, as the operator's `saya
/// coding resume` does.
pub(crate) async fn resume(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
) -> Arc<saya_cli::NativeCodingJob> {
    let approval = CodingAuthorization::scoped(
        &fixture
            .application
            .propose_resume(job.job_id())
            .await
            .unwrap(),
    )
    .unwrap();
    fixture
        .application
        .resume(job.job_id(), &approval, CancellationToken::new())
        .await
        .unwrap()
}

pub(crate) async fn checkout_path(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
    checkout: &str,
) -> PathBuf {
    fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap()
        .items
        .iter()
        .find(|record| record.ownership.managed_checkout_id().as_str() == checkout)
        .map(|record| PathBuf::from(record.ownership.managed_path().as_str()))
        .unwrap_or_else(|| panic!("checkout {checkout} exists"))
}

pub(crate) async fn checkouts_named(
    fixture: &Fixture,
    job: &saya_cli::NativeCodingJob,
    prefix: &str,
) -> usize {
    fixture
        .store
        .list_managed_checkouts(job.job_id(), None, 100)
        .await
        .unwrap()
        .items
        .iter()
        .filter(|record| {
            record
                .ownership
                .managed_checkout_id()
                .as_str()
                .starts_with(prefix)
        })
        .count()
}
