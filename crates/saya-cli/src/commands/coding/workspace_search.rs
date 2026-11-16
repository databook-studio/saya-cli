//! Read policy comes from current verified owned artifacts, never tool arguments.
use super::checks::ChildCodingOperations;
use saya_agent::{CancellationToken, ToolError};
use saya_harness::coding::{
    snapshot::{SnapshotManifest, SnapshotOmissionReason},
    worktree::verify_owned_checkout,
};

fn unavailable() -> ToolError {
    ToolError::Runner("job search private policy could not be verified".into())
}
impl ChildCodingOperations {
    pub(crate) async fn search_private_prefixes(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<String>, ToolError> {
        // An in-place job's baseline is an edit-time marker with no inventory,
        // so it names no private prefix. Search's own sensitive-path refusal
        // and its `.git` exclusion cover the policy the walk used to list.
        if self.application.is_in_place() {
            let verified = self.in_place_root()?;
            if verified.checkout_id() != &self.checkout
                || verified.snapshot_id() != self.authority.snapshot_id()
            {
                return Err(unavailable());
            }
            if cancellation.is_cancelled() {
                return Err(ToolError::Runner("job search was cancelled".into()));
            }
            return Ok(Vec::new());
        }
        let verified =
            verify_owned_checkout(&self.ownership().map_err(|_| unavailable())?, cancellation)
                .await
                .map_err(|_| unavailable())?;
        // W is agent-less: the root ceiling and every helper read the same
        // checkout, so its identity is the snapshot it was materialized
        // from, never an agent id.
        if verified.agent_id().is_some() || verified.snapshot_id() != self.authority.snapshot_id() {
            return Err(unavailable());
        }
        let bytes = self
            .application
            .store
            .get_checkout_plan_bytes(&self.job, &self.checkout)
            .await
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        let plan = SnapshotManifest::from_verified_bytes(&bytes.plan_bytes, verified.plan_sha256())
            .map_err(|_| unavailable())?;
        let mut prefixes = Vec::new();
        let mut total = 0usize;
        for omission in plan.omissions() {
            if omission.reason() == SnapshotOmissionReason::ConfiguredPrivatePath {
                let path = omission.path().as_str();
                total = total.saturating_add(path.len());
                if path.len() > 512 || prefixes.len() >= 4096 || total > 256 * 1024 {
                    return Err(unavailable());
                }
                prefixes.push(path.to_owned());
            }
        }
        if cancellation.is_cancelled() {
            return Err(ToolError::Runner("job search was cancelled".into()));
        }
        Ok(prefixes)
    }
}
