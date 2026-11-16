//! Owned native deletion: trusted policy, command exclusion and joined blocking work.
use super::checks::ChildCodingOperations;
use super::{KnownWorkspaceMutation, KnownWorkspaceMutationOperation};
use saya_agent::{CancellationToken, ToolError};
use saya_harness::{
    coding::{
        snapshot::{SnapshotManifest, sensitive_path},
        worktree::{
            InPlaceRootInput, VerifiedCheckout, verify_admitted_checkout, verify_in_place_root,
        },
    },
    workspace::{ControlledRemoveError, MAX_IO_BYTES, Workspace},
};
use saya_types::{CodingFailureClass, JobState, Sha256Digest, WorkspacePath};
use serde_json::{Value, json};
use std::path::Path;

fn refused(message: &'static str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}
fn unavailable() -> ToolError {
    // Authoritative read failures are not promoted to an optimistic refusal.
    ToolError::Runner("job deletion authority could not be verified".into())
}
#[path = "workspace_delete/worker.rs"]
mod worker;

/// Whether `path` is outside what an in-place job may touch: the job's own
/// root scratch tree, any Git metadata component (case-insensitive), or a
/// credential-shaped path. This is the job's substitute for `git check-ignore`.
fn omitted_path(path: &str) -> bool {
    path == ".saya"
        || path.starts_with(".saya/")
        || path
            .split('/')
            .any(|component| component.eq_ignore_ascii_case(".git"))
        || sensitive_path(path)
}

#[cfg(all(test, unix))]
#[path = "workspace_delete_tests.rs"]
mod tests;

impl ChildCodingOperations {
    /// The in-place root re-verified at use time. The private root record
    /// names the baseline by plan digest, which `verify_in_place_root`
    /// authenticates, so a rewritten baseline is refused rather than trusted.
    pub(super) fn in_place_root(&self) -> Result<VerifiedCheckout, ToolError> {
        let record =
            super::root_checkout::InPlaceRootRecord::read(&self.records.join("inplace-root.json"))
                .map_err(|_| unavailable())?;
        let input = InPlaceRootInput {
            artifacts: &self.application.artifacts,
            state_root: &self.application.state_root,
            root_id: record.root_id(),
            root_path: Path::new(record.root_path()),
            snapshot_id: record.snapshot_id(),
            plan_sha256: record.plan_sha256(),
        };
        verify_in_place_root(&input).map_err(|_| unavailable())
    }

    pub(crate) async fn delete_workspace(
        &self,
        path: WorkspacePath,
        expected: Sha256Digest,
        cancellation: CancellationToken,
    ) -> Result<Value, ToolError> {
        self.authority
            .admit_workspace_write(&self.checkout)
            .map_err(|_| refused("the child authority does not admit this deletion"))?;
        if cancellation.is_cancelled() {
            return Err(refused("deletion was cancelled before dispatch"));
        }
        let status = self
            .application
            .store
            .coding_recovery_status(&self.job)
            .await
            .map_err(|_| unavailable())?;
        if status.state != JobState::Running
            || status.authority.as_ref().is_none_or(|current| {
                current.epoch != self.authority.revocation_epoch()
                    || &current.policy_digest != self.authority.policy_digest()
            })
        {
            return Err(refused("deletion runtime authority is no longer current"));
        }
        // The root ceiling (authority view agent "coordinator") is the coordinator
        // itself, not a registered child: the child-currency check does not apply.
        // Children still must match a currently-running child row, unchanged.
        let is_root_ceiling = self.authority.agent_id().as_str() == "coordinator";
        if !is_root_ceiling {
            let children = self
                .application
                .store
                .list_coding_children(&self.job, None, 100)
                .await
                .map_err(|_| unavailable())?;
            if children.next_cursor.is_some()
                || !children.items.iter().any(|child| {
                    &child.agent_id == self.authority.agent_id()
                        && &child.current_invocation_id == self.authority.invocation_id()
                        && child.state == JobState::Running
                })
            {
                return Err(refused("deletion child invocation is no longer current"));
            }
        }
        if self
            .application
            .store
            .coding_cancel_requested(&self.job)
            .await
            .map_err(|_| unavailable())?
        {
            return Err(refused("deletion was cancelled before dispatch"));
        }
        if cancellation.is_cancelled() {
            return Err(refused("deletion was cancelled before dispatch"));
        }
        // An in-place root has no managed checkout row and no Git ignore
        // rules: it is verified from the job's private baseline record, and
        // the policy predicate below decides what deletion may not touch.
        let verified = if self.application.is_in_place() {
            self.in_place_root()?
        } else {
            verify_admitted_checkout(&self.ownership().map_err(|_| unavailable())?, &cancellation)
                .await
                .map_err(|error| match error {
                    saya_harness::coding::worktree::OwnershipError::RepositoryMismatch
                        if cancellation.is_cancelled() =>
                    {
                        refused("deletion was cancelled before dispatch")
                    }
                    saya_harness::coding::worktree::OwnershipError::Git(
                        saya_harness::coding::git::GitError::Cancelled,
                    ) => refused("deletion was cancelled before dispatch"),
                    _ => unavailable(),
                })?
        };
        // The shared working checkout W is agent-less: the root ceiling and
        // every helper write the same checkout, so the durable checkout
        // agent is `None` for all of them and only the in-memory authority view
        // names an actor. The checkout identity is therefore the snapshot it
        // was materialized from, not an agent; the calling child's currency was
        // proved above.
        if verified.checkout_id() != &self.checkout
            || verified.agent_id().is_some()
            || verified.snapshot_id() != self.authority.snapshot_id()
        {
            return Err(refused("deletion checkout does not match this invocation"));
        }
        if self.application.is_in_place() {
            if omitted_path(path.as_str()) {
                return Err(refused("deletion target is omitted by job policy"));
            }
        } else {
            let plan = self
                .application
                .store
                .get_checkout_plan_bytes(&self.job, &self.checkout)
                .await
                .map_err(|_| unavailable())?
                .ok_or_else(unavailable)?;
            let plan =
                SnapshotManifest::from_verified_bytes(&plan.plan_bytes, verified.plan_sha256())
                    .map_err(|_| unavailable())?;
            if plan.omissions().iter().any(|omission| {
                let omitted = omission.path().as_str();
                path.as_str() == omitted
                    || path
                        .as_str()
                        .strip_prefix(omitted)
                        .is_some_and(|tail| tail.starts_with('/'))
            }) {
                return Err(refused("deletion target is omitted by job policy"));
            }
            if self
                .application
                .service
                .owned_path_is_ignored(&self.checkout, &path, &cancellation)
                .await
                .map_err(|error| match error {
                    saya_harness::coding::git::GitError::Cancelled => {
                        refused("deletion was cancelled before dispatch")
                    }
                    saya_harness::coding::git::GitError::TimedOut => {
                        refused("deletion policy deadline reached before dispatch")
                    }
                    _ => unavailable(),
                })?
            {
                return Err(refused("deletion target is ignored by job policy"));
            }
        }
        let workspace =
            Workspace::open(Path::new(verified.path().as_str())).map_err(|_| unavailable())?;
        // An in-place job records the deleted bytes as a first touch before
        // the removal, but only when they still match the digest the caller
        // stated. A bound or a read failure leaves the delete unrecorded; it
        // never blocks the removal.
        let mut before_image_recorded = false;
        if let Some(store) = &self.before_images
            && let Ok(file) = workspace.read(path.as_str(), MAX_IO_BYTES as u64)
            && file.digest == expected.as_str()
        {
            let executable = workspace.is_executable(path.as_str()).unwrap_or(false);
            before_image_recorded = matches!(
                store.record_first_touch(
                    &path,
                    Some(&file.bytes),
                    executable,
                    self.before_image_turn()()
                ),
                Ok(saya_harness::coding::local::before_image::FirstTouch::Recorded)
            );
        }
        let removed_path = path.as_str().to_owned();
        let removed_digest = expected.as_str().to_owned();
        let journal_cancellation = cancellation.clone();
        let result = worker::run(workspace, path, expected, cancellation).await;
        match result {
            Ok(()) => {
                if self.application.is_in_place() {
                    let committed = KnownWorkspaceMutation::new(
                        KnownWorkspaceMutationOperation::Delete,
                        removed_path.clone(),
                        removed_digest.clone(),
                        0,
                    )
                    .map_err(|_| unavailable())?;
                    if self.record_known_commit(&committed).is_err() {
                        self.fail_owner_operation(
                            CodingFailureClass::MandatoryPersistence,
                            None,
                            &journal_cancellation,
                        );
                        return Err(ToolError::Runner(
                            "workspace deletion committed but its known-commit receipt could not be written"
                                .into(),
                        ));
                    }
                }
                Ok(json!({"path":removed_path,"removed":true,"removed_digest":removed_digest}))
            }
            Err(error @ ControlledRemoveError::NoEffect(_)) => {
                if before_image_recorded && let Some(store) = &self.before_images {
                    let _ = store.forget(&removed_path);
                }
                Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                    error.to_string(),
                ))))
            }
            _ => Err(ToolError::Runner(
                "workspace deletion outcome could not be confirmed".into(),
            )),
        }
    }
}
