use super::{CodingApplication, CodingJobRequest};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::JobBudgetLedger, context::OriginalProjectContext, snapshot::SnapshotManifest,
};
use saya_types::{RepositoryId, Sha256Digest};
use std::{path::PathBuf, sync::Arc};

/// The one live owner of an approved native job. No grants are deserialized.
pub struct NativeCodingJob {
    pub(super) application: Arc<CodingApplication>,
    pub(super) request: CodingJobRequest,
    pub(super) plan: SnapshotManifest,
    pub(super) repository_id: RepositoryId,
    pub(super) policy_digest: Sha256Digest,
    pub(super) epoch: u64,
    pub(super) ledger: JobBudgetLedger,
    pub(super) project: OriginalProjectContext,
    pub(super) records: PathBuf,
    pub(super) root_steering: saya_agent::AgentSteering,
    pub(super) root_mailbox: tokio::sync::Mutex<Option<saya_agent::AgentMailbox>>,
    pub(super) observation_sequence: std::sync::atomic::AtomicU64,
    pub(super) observation_failed: std::sync::atomic::AtomicBool,
    pub(super) child_failed: std::sync::atomic::AtomicBool,
    pub(super) command_owner: Arc<saya_harness::coding::commands::CommandOwner>,
    pub(super) active: tokio::sync::Mutex<Option<super::controls::LiveSchedule>>,
    pub(super) progress:
        tokio::sync::broadcast::Sender<saya_harness::coding::coordinator::ChildEvent>,
    pub(super) cancellation: CancellationToken,
}
impl NativeCodingJob {
    pub fn subscribe(
        &self,
    ) -> tokio::sync::broadcast::Receiver<saya_harness::coding::coordinator::ChildEvent> {
        self.progress.subscribe()
    }
    pub fn job_id(&self) -> &saya_types::JobId {
        self.request.job_id()
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
