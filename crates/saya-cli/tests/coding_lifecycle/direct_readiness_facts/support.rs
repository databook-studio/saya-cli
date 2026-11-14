//! The direct-mode fixtures: a direct job over the fixture repository
//! (optionally with an extra baseline file), its scripted run, and the store
//! reads the readiness-facts and rewind tests assert against.

use crate::provider::Provider;
use crate::support::Fixture;
use saya_agent::{AgentOutput, CancellationToken, NoopEventSink};
use saya_cli::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use saya_store::CodingCompletion;
use saya_types::{AgentId, CodingFailureClass, JobId, JobState};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Bound on one scripted run.
pub(crate) const RUN: Duration = Duration::from_secs(90);

/// One readiness-facts scenario: the scripted mode, the approved write paths,
/// and the two optional fixture knobs (an extra committed baseline file, and
/// transport retry delays).
pub(crate) struct Setup {
    label: &'static str,
    mode: u8,
    write_paths: Vec<String>,
    retry_delays: Vec<u64>,
    baseline: Vec<(&'static str, &'static str)>,
}

impl Setup {
    pub(crate) fn new(label: &'static str, mode: u8, write_paths: Vec<String>) -> Self {
        Self {
            label,
            mode,
            write_paths,
            retry_delays: Vec::new(),
            baseline: Vec::new(),
        }
    }

    /// Transport retry delays: a cut child stream needs one retry to exhaust.
    pub(crate) fn retry(mut self, delays: Vec<u64>) -> Self {
        self.retry_delays = delays;
        self
    }

    /// Files committed into the repository before the job's baseline snapshot,
    /// so a scenario can delete one of them.
    pub(crate) fn baseline(mut self, files: Vec<(&'static str, &'static str)>) -> Self {
        self.baseline = files;
        self
    }
}

/// Opens a direct-mode job without running it.
pub(crate) async fn facts_job(setup: Setup) -> (Fixture, Arc<NativeCodingJob>, Provider) {
    let provider = Provider::script(false, setup.mode);
    let mut fixture = Fixture::with_endpoint(setup.label, Some(&provider.endpoint)).await;
    for (path, content) in &setup.baseline {
        commit_baseline(&fixture, path, content);
    }
    fixture.runtime.resolved.limits.job.logical_requests = finite(400);
    fixture.runtime.resolved.ai.retry_delays_ms = setup.retry_delays;
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
    let request = CodingJobRequest::new(
        JobId::parse(setup.label).unwrap(),
        "Direct-mode readiness facts with helpers".into(),
        setup.write_paths,
    )
    .unwrap()
    .with_tools_allow(Some(
        [
            "helpers",
            "read",
            "write",
            "edit",
            "glob",
            "grep",
            "shell",
            "checklist",
            "delete",
            "rewind",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    ))
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    let job = fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    (fixture, job, provider)
}

/// Commits one extra file into the repository, so the job's baseline snapshot
/// holds it and a later deletion is a real per-path change.
fn commit_baseline(fixture: &Fixture, path: &str, content: &str) {
    std::fs::write(fixture.repository.join(path), content).unwrap();
    for args in [
        ["add", path].as_slice(),
        ["commit", "--quiet", "-m", "extra"].as_slice(),
    ] {
        let status = std::process::Command::new(&fixture.git)
            .current_dir(&fixture.repository)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "the fixture baseline commits {path}");
    }
}

/// Runs a direct-mode job to completion and returns its own result.
pub(crate) async fn run(job: &Arc<NativeCodingJob>) -> AgentOutput {
    tokio::time::timeout(RUN, job.run(&NoopEventSink))
        .await
        .expect("the scripted run terminates")
        .unwrap()
}

/// The advisory readiness the job reports — the `saya coding show` surface.
/// The `coding_status` tool does not exist, so the facts a status read carries
/// are read here.
pub(crate) async fn readiness(job: &Arc<NativeCodingJob>) -> Value {
    crate::direct_task::readiness_now(job).await
}

/// The child's latest durable completion, or `None` when it has none.
pub(crate) async fn latest_completion(
    fixture: &Fixture,
    job: &NativeCodingJob,
    agent: &str,
) -> Option<CodingCompletion> {
    let agent = AgentId::parse(agent).unwrap();
    fixture
        .store
        .list_coding_completion_history(job.job_id(), None, 32)
        .await
        .unwrap()
        .items
        .into_iter()
        .rfind(|item| item.result.agent_id() == &agent)
}

/// The class the child's own durable completion records, or `None` when its
/// completion carries none.
pub(crate) fn completion_class(
    completion: &Option<CodingCompletion>,
) -> Option<CodingFailureClass> {
    completion
        .as_ref()
        .and_then(|item| item.result.failure_class())
}

/// Waits until the store reports `state` for `agent`, so a test can let the run
/// continue only once the child has actually failed.
pub(crate) async fn wait_for_child_state(
    fixture: &Fixture,
    job: &NativeCodingJob,
    agent: &str,
    state: JobState,
) {
    let deadline = Instant::now() + RUN;
    loop {
        let children = fixture
            .store
            .list_coding_children(job.job_id(), None, 100)
            .await
            .unwrap()
            .items;
        if children
            .iter()
            .any(|child| child.agent_id.as_str() == agent && child.state == state)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the child {agent} never reached {state:?}: {children:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn finite(value: u64) -> saya_config::BudgetLimit {
    saya_config::BudgetLimit::Finite(std::num::NonZeroU64::new(value).unwrap())
}
