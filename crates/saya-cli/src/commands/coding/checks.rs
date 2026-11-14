use super::CodingApplication;
use saya_harness::coding::{policy::ChildAuthorityView, worktree::OwnershipInput};
use saya_types::{JobId, ManagedCheckoutId};
use std::path::PathBuf;
use std::sync::Arc;

/// One child-bound adapter. Every effect reopens owned checkout evidence; no
/// model argument selects its job, checkout, config, or executable directory.
pub(crate) struct ChildCodingOperations {
    pub(crate) application: Arc<CodingApplication>,
    pub(crate) job: JobId,
    pub(crate) checkout: ManagedCheckoutId,
    pub(crate) authority: ChildAuthorityView,
    pub(crate) ledger: saya_harness::coding::budget::JobBudgetLedger,
    pub(crate) records: PathBuf,
    /// The job's shared before-image store, for an in-place job.
    pub(crate) before_images:
        Option<Arc<saya_harness::coding::local::before_image::BeforeImageStore>>,
    pub(crate) owner_failure: Arc<super::NativeOwnerFailureState>,
    pub(crate) failed: Arc<std::sync::atomic::AtomicBool>,
}
impl ChildCodingOperations {
    /// The job's rewind clock as the before-image turn: one past the highest
    /// recorded checkpoint. An unreadable rewind store reads as turn zero;
    /// the turn is a label, not a policy.
    pub(crate) fn before_image_turn(&self) -> Arc<dyn Fn() -> u32 + Send + Sync> {
        let records = self.records.clone();
        Arc::new(move || {
            super::rewind::RewindStore::open(&super::rewind::store_root(&records))
                .next_turn()
                .unwrap_or(0)
        })
    }

    pub(crate) fn ownership(&self) -> Result<OwnershipInput<'_>, super::CodingError> {
        let host = &self.application;
        Ok(OwnershipInput {
            store: &host.store,
            service: &host.service,
            repository: host.git_repository("job operations")?,
            artifacts: &host.artifacts,
            job_id: &self.job,
            checkout_id: &self.checkout,
        })
    }
}
