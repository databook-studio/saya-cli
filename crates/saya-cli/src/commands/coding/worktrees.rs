use super::{CodingApplication, CodingError};
use saya_agent::CancellationToken;
use saya_harness::coding::worktree::{
    OwnershipError, OwnershipInput, VerifiedCheckout, verify_admitted_checkout,
};
use saya_types::JobId;
use serde_json::{Value, json};
impl CodingApplication {
    pub async fn worktrees(&self, job: &JobId) -> Result<Value, CodingError> {
        self.status(job).await?;
        let page = self
            .store
            .list_managed_checkouts(job, None, 100)
            .await
            .map_err(CodingError::owner)?;
        let mut listed = json!({"checkouts":page.items.iter().map(|record|json!({"checkout_id":record.ownership.managed_checkout_id(),"lifecycle":record.current_lifecycle})).collect::<Vec<_>>(),"next_cursor":page.next_cursor});
        // An in-place root has no managed worktree: the listing names the
        // folder instead, and a Git listing keeps its exact published shape.
        if let Some(local) = self.local_root() {
            listed["mode"] = json!("in_place");
            listed["root"] = json!({
                "path": local.path().to_string_lossy(),
                "root_id": local.id().as_str(),
                "git": false,
            });
        }
        Ok(listed)
    }
}

impl super::NativeCodingJob {
    pub(super) fn ownership<'a>(
        &'a self,
        checkout: &'a saya_types::ManagedCheckoutId,
    ) -> Result<saya_harness::coding::worktree::OwnershipInput<'a>, CodingError> {
        let app = &self.application;
        Ok(saya_harness::coding::worktree::OwnershipInput {
            store: &app.store,
            service: &app.service,
            repository: app.git_repository("working checkout")?,
            artifacts: &app.artifacts,
            job_id: self.job_id(),
            checkout_id: checkout,
        })
    }
    pub async fn create_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<Option<VerifiedCheckout>, CodingError> {
        use saya_harness::coding::worktree::{CreateCheckoutInput, create_managed_checkout};
        let app = &self.application;
        let snapshot = app
            .artifacts
            .read_snapshot(self.plan.snapshot_id())
            .map_err(CodingError::owner)?;
        let requested = snapshot
            .entries()
            .iter()
            .map(|entry| entry.captured_bytes())
            .sum();
        saya_harness::coding::worktree::ensure_retention_capacity(
            &app.store,
            &app.service,
            self.job_id(),
            requested,
            1024 * 1024 * 1024,
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;

        let plan = self.plan.git().ok_or(CodingError::InvalidRequest)?;
        let created = create_managed_checkout(
            &CreateCheckoutInput {
                store: &app.store,
                sequencer: &self.ledger.sequencer(),
                service: &app.service,
                repository: app.git_repository("checkout")?,
                artifacts: &app.artifacts,
                plan,
                job_id: self.job_id(),
                agent_id: None,
                checkout_id: checkout,
                base_commit: plan.base_commit(),
            },
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;
        Ok(created.into_verified())
    }
    pub async fn remove_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<(), CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        use saya_harness::coding::worktree::{CleanupInput, cleanup_managed_checkout};
        let lease = self
            .ledger
            .try_quiescent()
            .await
            .map_err(CodingError::owner)?;
        cleanup_managed_checkout(
            &CleanupInput {
                ownership: self.ownership(checkout)?,
                sequencer: &self.ledger.sequencer(),
                quiescence: &lease,
            },
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)
    }
    pub async fn retain_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<(), CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        saya_harness::coding::worktree::retain_managed_checkout(
            &self.ownership(checkout)?,
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;
        Ok(())
    }
    pub async fn reconcile_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<Value, CodingError> {
        let _operation = self.operation.try_lock().map_err(|_| CodingError::Busy)?;
        use saya_harness::coding::worktree::{CleanupInput, reconcile_managed_checkout};
        let lease = self
            .ledger
            .try_quiescent()
            .await
            .map_err(CodingError::owner)?;
        let result = reconcile_managed_checkout(
            &CleanupInput {
                ownership: self.ownership(checkout)?,
                sequencer: &self.ledger.sequencer(),
                quiescence: &lease,
            },
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;
        Ok(json!({"reconciliation":format!("{result:?}")}))
    }
}

/// Verifies the root's working checkout W for a root-over-W operation.
///
/// W is the job's one shared working checkout: the direct-mode root and every
/// helper write it directly, and it stays `Active` for the whole run —
/// nothing integrates into it, so only the operator's apply-back moves
/// it to `Retained`. Admits `Active | Retained` where `verify_owned_checkout`
/// (Active only) would refuse a W the operator has already applied or
/// discarded. The durable checkout agent stays `None` and only the in-memory
/// authority view carries "coordinator".
pub(crate) async fn verify_root_checkout<'a>(
    ownership: &OwnershipInput<'a>,
    cancellation: &CancellationToken,
) -> Result<VerifiedCheckout, OwnershipError> {
    verify_admitted_checkout(ownership, cancellation).await
}
