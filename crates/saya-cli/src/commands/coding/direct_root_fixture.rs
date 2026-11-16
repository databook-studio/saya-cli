//! Shared fixture for the direct-mode root-checkout tests: a direct job with
//! its agent-less working checkout W, and finished helpers whose writes are in
//! W (a helper writes the root's checkout directly, so there is no
//! per-helper checkout and no merge).
//!
//! Reuses the delegate-direct fixture for the job shape.

use super::{CodingTask, NativeCodingJob};

#[cfg(unix)]
use super::delegate_direct_fixture::{
    fixture as build_job, fixture_with_endpoint as build_endpoint_job,
    fixture_with_endpoint_and_agents as build_endpoint_agents_job,
};

#[cfg(unix)]
use super::create::digest;
#[cfg(unix)]
use saya_store::NewCodingChild;
#[cfg(unix)]
use saya_types::{
    AgentId, CodingEventKind, CodingEventV1, CodingResultOutcome, InvocationId, JobState,
    JoinedResultV1, ManagedCheckoutId, Sha256Digest,
};
#[cfg(unix)]
use std::{fs, num::NonZeroU64, path::PathBuf, sync::Arc};

#[cfg(unix)]
pub(super) struct RootFixture {
    pub(super) root: PathBuf,
    pub(super) job: Arc<NativeCodingJob>,
}

#[cfg(unix)]
impl Drop for RootFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
pub(super) fn agent(value: &str) -> AgentId {
    AgentId::parse(value).unwrap()
}

/// Builds a direct-mode job with its agent-less root working checkout W.
#[cfg(unix)]
pub(super) async fn fixture(case: &str, write_paths: Vec<String>) -> RootFixture {
    root_fixture(build_job(case, write_paths).await).await
}

/// The same fixture with its model endpoint pointed at `endpoint` (a local
/// scripted provider) and, when given, a `coding.max_children` bound, so a
/// test can drive the job's helper children through a controllable transport.
#[cfg(unix)]
pub(super) async fn fixture_with_endpoint(
    case: &str,
    write_paths: Vec<String>,
    endpoint: &str,
    max_children: Option<usize>,
) -> RootFixture {
    root_fixture(build_endpoint_job(case, write_paths, endpoint, max_children).await).await
}

/// The same fixture with an `[agents]` table resolved into the job — the
/// registry and the child bound — so a test can drive a configured helper type
/// through the whole direct path.
#[cfg(unix)]
pub(super) async fn fixture_with_endpoint_and_agents(
    case: &str,
    write_paths: Vec<String>,
    endpoint: &str,
    agents_toml: &str,
) -> RootFixture {
    root_fixture(build_endpoint_agents_job(case, write_paths, endpoint, agents_toml).await).await
}

#[cfg(unix)]
async fn root_fixture(base: super::delegate_direct_fixture::Fixture) -> RootFixture {
    base.job.ensure_root_checkout().await.unwrap();
    let root = base.root.clone();
    let job = base.job.clone();
    // RootFixture owns the cleanup; forget `base` so its Drop does not remove
    // the shared root directory out from under the returned fixture.
    std::mem::forget(base);
    RootFixture { root, job }
}

/// The root working checkout W's filesystem path.
#[cfg(unix)]
pub(super) fn root_path(job: &NativeCodingJob) -> PathBuf {
    let id = root_checkout_id(job);
    PathBuf::from(
        job.application
            .service
            .managed_path(&id)
            .unwrap()
            .as_str()
            .to_owned(),
    )
}

/// Writes `bytes` to `path` inside the root working checkout W.
#[cfg(unix)]
pub(super) fn edit_root(job: &NativeCodingJob, path: &str, bytes: &[u8]) {
    fs::write(root_path(job).join(path), bytes).unwrap();
}

#[cfg(unix)]
fn root_identity(job: &NativeCodingJob) -> Sha256Digest {
    digest(format!("{}:root", job.job_id().as_str()).as_bytes()).unwrap()
}

/// The root's working checkout id W (`work-<digest[:32]>`).
#[cfg(unix)]
pub(super) fn root_checkout_id(job: &NativeCodingJob) -> ManagedCheckoutId {
    ManagedCheckoutId::parse(&format!("work-{}", &root_identity(job).as_str()[..32])).unwrap()
}

/// The root coordinator's invocation id (`inv-<digest[:32]>`).
#[cfg(target_os = "macos")]
pub(super) fn root_invocation(job: &NativeCodingJob) -> InvocationId {
    InvocationId::parse(&format!("inv-{}", &root_identity(job).as_str()[..32])).unwrap()
}

/// The child identity digest its invocation id derives from.
#[cfg(unix)]
pub(super) fn child_identity(job: &NativeCodingJob, agent: &str) -> Sha256Digest {
    digest(format!("{}:{}", job.job_id().as_str(), agent).as_bytes()).unwrap()
}

/// Creates one finished helper whose work is in W: its durable row, a task in
/// `tasks.jsonl`, a terminal completion, and (when `edit` is given) its bytes
/// written into the root working checkout W. Returns the sequence of the
/// completion it recorded.
#[cfg(unix)]
pub(super) async fn helper_wrote_w(
    job: &NativeCodingJob,
    agent_name: &str,
    write_paths: Vec<String>,
    outcome: CodingResultOutcome,
    answer: &str,
    edit: Option<(&str, &[u8])>,
) -> NonZeroU64 {
    let agent_id = agent(agent_name);
    let invocation = InvocationId::parse(&format!(
        "inv-{}",
        &child_identity(job, agent_name).as_str()[..32]
    ))
    .unwrap();
    job.application
        .store
        .create_coding_child(NewCodingChild {
            job_id: job.job_id().clone(),
            agent_id: agent_id.clone(),
            current_invocation_id: invocation.clone(),
        })
        .await
        .unwrap();
    append_task(job, &agent_id, write_paths);
    if let Some((path, bytes)) = edit {
        edit_root(job, path, bytes);
    }
    record_completion(job, &agent_id, &invocation, outcome, answer).await
}

#[cfg(unix)]
fn append_task(job: &NativeCodingJob, agent: &AgentId, write_paths: Vec<String>) {
    let task = CodingTask {
        agent_id: agent.clone(),
        task: format!("write for {}", agent.as_str()),
        subagent_type: "general".into(),
        write_paths,
        dependencies: vec![],
    };
    let record = serde_json::to_vec(&[task]).unwrap();
    super::context::append_line(
        &job.records.join("tasks.jsonl"),
        &record,
        super::task_history::MAX_TASK_HISTORY_BYTES,
    )
    .unwrap();
}

#[cfg(unix)]
async fn record_completion(
    job: &NativeCodingJob,
    agent: &AgentId,
    invocation: &InvocationId,
    outcome: CodingResultOutcome,
    answer: &str,
) -> NonZeroU64 {
    // Route the child's lifecycle events through the parent job's sequencer so
    // its event counter stays in lock-step with the durable stream.
    let job_id = job.job_id().clone();
    job.ledger
        .sequencer()
        .append_event(|sequence| {
            CodingEventV1::new(
                NonZeroU64::new(sequence).unwrap(),
                job_id.clone(),
                Some(agent.clone()),
                Some(invocation.clone()),
                CodingEventKind::StateChanged {
                    state: JobState::Running,
                },
            )
            .unwrap()
        })
        .await
        .unwrap();
    let result = JoinedResultV1::new(
        job_id.clone(),
        agent.clone(),
        invocation.clone(),
        outcome,
        answer.to_string(),
        false,
    )
    .unwrap();
    job.ledger
        .sequencer()
        .append_completion(&result)
        .await
        .unwrap()
        .sequence
}

/// Build `ChildCodingOperations` over `checkout`. The checkout is verified
/// ADMITTED (Active | Retained) so a retained W composes; `delete_workspace`
/// then exercises the per-operation Active-only gate.
// Child operations are composed over a confined checkout, which only a host
// that proves host confinement can supply; the Linux probe fails closed.
#[cfg(target_os = "macos")]
pub(super) async fn operations(
    job: &NativeCodingJob,
    checkout: &ManagedCheckoutId,
    agent_id: &AgentId,
    invocation: &InvocationId,
) -> Arc<super::checks::ChildCodingOperations> {
    use super::approval::ChildIssuanceApproval;
    use saya_harness::coding::policy::{
        ChildAuthorityRequest, ChildRole, RootDelegationInput, RootDelegationScope,
    };
    use saya_harness::coding::worktree::{
        ScratchScope, confine_checkout, verify_admitted_checkout,
    };
    use saya_types::Capabilities;
    use std::sync::atomic::AtomicBool;

    let app = &job.application;
    let verified = verify_admitted_checkout(&job.ownership(checkout).unwrap(), &job.cancellation)
        .await
        .unwrap();
    let confined = confine_checkout(&verified, &app.program_dir, ScratchScope::Root).unwrap();
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    let input = RootDelegationInput {
        job_id: job.job_id().clone(),
        agent_id: agent_id.clone(),
        invocation_id: invocation.clone(),
        repository_id: job.repository_id.clone(),
        snapshot_id: job.plan.snapshot_id().clone(),
        managed_checkout_id: Some(checkout.clone()),
        policy_digest: job.policy_digest.clone(),
        revocation_epoch: job.epoch,
        capabilities: capabilities.clone(),
        database_profiles: Vec::new(),
    };
    let approval = ChildIssuanceApproval {
        expected: input.approval_request(),
    };
    let authority = RootDelegationScope::approve(input, &approval)
        .await
        .unwrap()
        .narrow(
            ChildAuthorityRequest::new(ChildRole::Implementer, capabilities, Vec::new()).unwrap(),
            confined.platform_support(),
        )
        .unwrap();
    Arc::new(super::checks::ChildCodingOperations {
        application: Arc::clone(app),
        job: job.job_id().clone(),
        checkout: checkout.clone(),
        authority,
        ledger: job.ledger.clone(),
        records: job.records.clone(),
        before_images: job.before_images(),
        owner_failure: Arc::new(super::NativeOwnerFailureState::default()),
        failed: Arc::new(AtomicBool::new(false)),
    })
}
