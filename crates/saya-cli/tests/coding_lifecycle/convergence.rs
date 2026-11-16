//! A coordinator whose calls keep failing is ended by the owner's stall bound,
//! and a delegation either registers every child it names or none. The
//! scenarios run under a direct-mode coordinator: readiness is advisory there,
//! so a test asserts the facts (`readiness`) instead of a forced `next_action`.
use super::{provider::Provider, support::Fixture};
use saya_agent::CancellationToken;
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_types::JobId;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

#[path = "convergence/progress_retry.rs"]
mod progress_retry;
#[path = "convergence/regression.rs"]
mod regression;
#[path = "convergence/stall_bound.rs"]
mod stall_bound;
#[path = "convergence/unscheduled_child.rs"]
mod unscheduled_child;

/// Consecutive failed, refused or denied root tool calls before the owner ends
/// a run whose next action is `finish`.
pub(super) const STOP_AFTER: usize = 12;

/// Three times `STOP_AFTER`: a mid-point a long run of successful calls must
/// outlast before the owner's own stall bound is in sight.
pub(super) const STALL_AFTER: usize = 3 * STOP_AFTER;

/// The owner's stall bound for a run, as `convergence.rs` defines it: eight
/// times `STOP_AFTER`.
pub(super) const DIRECT_STALL_AFTER: usize = 8 * STOP_AFTER;

/// The group's direct fixture: the fixture's runtime with the logical-request
/// ceiling raised well above the default 80, so a scripted session that
/// outlasts the stall bound fits. `write_job` opens a direct job over it.
pub(super) async fn direct_fixture(label: &str, provider: &Provider) -> Fixture {
    let mut fixture = Fixture::with_endpoint(label, Some(&provider.endpoint)).await;
    fixture.runtime.resolved.limits.job.logical_requests = finite(400);
    // These scenarios script long runs of deliberately identical calls to
    // exercise the owner's stall bound, so the job's own progress policy is
    // off; `progress_guards.rs` tests that policy on.
    fixture.runtime.resolved.limits.job.no_progress_turns = 0;
    fixture.runtime.resolved.limits.job.repeat_limit = 0;
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

pub(super) async fn write_job(
    fixture: &Fixture,
    label: &str,
    paths: &[&str],
) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(label).unwrap(),
        "Make the native child changes with helpers".into(),
        paths.iter().map(|path| (*path).into()).collect(),
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

/// Every tool result the scripted root has received, by call tag.
pub(super) fn tool_results(provider: &Provider) -> BTreeMap<String, Value> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .flat_map(|body| {
            let request: Value = serde_json::from_str(body).unwrap();
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|message| {
                    let id = message["tool_call_id"].as_str()?;
                    Some((
                        id.to_owned(),
                        serde_json::from_str(message["content"].as_str()?).ok()?,
                    ))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The job's child checkouts. A direct job also owns the agent-less working
/// checkout W (`work-…`) and may own prepared candidates (`candidate-…`), so
/// only the `child-…` records count as delegation checkouts.
pub(super) async fn child_checkouts(fixture: &Fixture, job: &NativeCodingJob) -> usize {
    crate::direct_prepare::fixture::checkouts_named(fixture, job, "child-").await
}

/// Requests the coordinator itself made, apart from its children's.
pub(super) fn root_requests(provider: &Provider) -> usize {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .filter(|body| !body.contains("UNTRUSTED CHILD TASK"))
        .count()
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}
