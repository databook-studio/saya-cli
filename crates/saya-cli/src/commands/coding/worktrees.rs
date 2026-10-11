use super::{CodingApplication, CodingError};
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
        Ok(
            json!({"checkouts":page.items.iter().map(|record|json!({"checkout_id":record.ownership.managed_checkout_id(),"lifecycle":record.current_lifecycle})).collect::<Vec<_>>(),"next_cursor":page.next_cursor}),
        )
    }
}

impl super::NativeCodingJob {
    pub(super) fn ownership<'a>(
        &'a self,
        checkout: &'a saya_types::ManagedCheckoutId,
    ) -> saya_harness::coding::worktree::OwnershipInput<'a> {
        let app = &self.application;
        saya_harness::coding::worktree::OwnershipInput {
            store: &app.store,
            service: &app.service,
            repository: &app.repository,
            artifacts: &app.artifacts,
            job_id: self.job_id(),
            checkout_id: checkout,
        }
    }
    pub async fn create_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<(), CodingError> {
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

        create_managed_checkout(
            &CreateCheckoutInput {
                store: &app.store,
                sequencer: &self.ledger.sequencer(),
                service: &app.service,
                repository: &app.repository,
                artifacts: &app.artifacts,
                plan: &self.plan,
                job_id: self.job_id(),
                agent_id: None,
                checkout_id: checkout,
                base_commit: self.plan.base_commit(),
            },
            &self.cancellation,
        )
        .await
        .map_err(CodingError::owner)?;
        Ok(())
    }
    pub async fn remove_checkout(
        &self,
        checkout: &saya_types::ManagedCheckoutId,
    ) -> Result<(), CodingError> {
        use saya_harness::coding::worktree::{CleanupInput, cleanup_managed_checkout};
        let lease = self
            .ledger
            .try_quiescent()
            .await
            .map_err(CodingError::owner)?;
        cleanup_managed_checkout(
            &CleanupInput {
                ownership: self.ownership(checkout),
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
        saya_harness::coding::worktree::retain_managed_checkout(
            &self.ownership(checkout),
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
        use saya_harness::coding::worktree::{CleanupInput, reconcile_managed_checkout};
        let lease = self
            .ledger
            .try_quiescent()
            .await
            .map_err(CodingError::owner)?;
        let result = reconcile_managed_checkout(
            &CleanupInput {
                ownership: self.ownership(checkout),
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
