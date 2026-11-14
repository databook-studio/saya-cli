//! The one rewind operation every adapter runs: list a job's checkpoints, or
//! put its working checkout back to one. Both `saya job rewind` and
//! `/job rewind` reach this, so the two surfaces cannot drift.
//!
//! A rewind refuses while the job is live. The durable job state is the check
//! — the same one `saya job reconcile` documents — so a job running in another
//! process is refused exactly like one running here, and a job interrupted
//! mid-run reads as live until it is reconciled.

use super::apply::{Before, restore};
use super::apply_baseline::{restore_baseline, restore_before_images};
use super::{RewindStore, store_root};
use crate::commands::coding::{
    CodingApplication, CodingError, CodingJobRequest, RootChange, context, job_changes,
    read_root_checkout, verify_root_checkout,
};
use saya_agent::CancellationToken;
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::local::before_image::BeforeImageStore;
use saya_harness::coding::worktree::{
    InPlaceRootInput, OwnershipInput, VerifiedCheckout, verify_in_place_root,
};
use saya_types::{JobId, JobState};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

impl CodingApplication {
    /// Lists this job's checkpoints, or rewinds its working checkout to one.
    /// Listing is always allowed; a rewind is refused while the job is live.
    pub async fn rewind(
        &self,
        job: &JobId,
        to: Option<u32>,
        force: bool,
    ) -> Result<Value, CodingError> {
        let record = self.owned_job(job).await?;
        let store = RewindStore::open(&store_root(&self.state_root.join(job.as_str())));
        let Some(turn) = to else {
            return listing(&store, job);
        };
        // The named checkpoint is resolved before the job's state is consulted:
        // a rewind to a turn this job never checkpointed names nothing to
        // restore, and that is the fact the operator needs first. Nothing is
        // written on either refusal.
        if !store
            .list()?
            .iter()
            .any(|checkpoint| checkpoint.turn == turn)
        {
            return Err(CodingError::RewindUnknownTurn { turn });
        }
        if record.state == JobState::Running {
            return Err(CodingError::RewindWhileRunning);
        }
        let verified = self.rewind_checkout(job).await?;
        let records = self.state_root.join(job.as_str());
        let scope = AssignedPaths::parse(self.rewind_request(job)?.write_paths)
            .map_err(CodingError::write_paths)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let changes = job_changes(
            &self.artifacts,
            &records,
            &verified,
            &ignored,
            &CancellationToken::new(),
        )?;
        let foreign = if force {
            Vec::new()
        } else {
            self.foreign_paths(job, &verified, &changes)?
        };
        let before = self
            .is_in_place()
            .then(|| BeforeImageStore::open(&records))
            .transpose()
            .map_err(CodingError::owner)?;
        let current: Vec<String> = changes.into_iter().map(|change| change.path).collect();
        let report = restore(
            &store,
            &PathBuf::from(verified.path().as_str()),
            &scope,
            turn,
            &current,
            None,
            Before {
                images: before.as_ref(),
                foreign: &foreign,
            },
        )?;
        store.record_rewind(
            turn,
            crate::interactive::session::continuation::now_unix_ms(),
        )?;
        Ok(json!({
            "job_id": job,
            "turn": turn,
            "restored_paths": report.restored,
            "removed_paths": report.removed,
            "out_of_scope_paths": report.out_of_scope,
            "foreign_paths": report.foreign,
            "not_restorable_paths": report.not_restorable,
            "authority": "none",
            "note": "the next run's context states which turn the working checkout was rewound to; a rewind restores file content only, never a command, download or database effect; shell-made changes are reported, never rewound",
        }))
    }

    /// Restores the job's working checkout to its captured baseline: every
    /// baseline file the job's `--write` scope admits is put back, and every
    /// path the checkout added after it (and the scope admits) is removed.
    /// Files outside the scope are reported and never touched. This is the undo
    /// of last resort for a folder with no Git history.
    pub async fn rewind_baseline(&self, job: &JobId, force: bool) -> Result<Value, CodingError> {
        let record = self.owned_job(job).await?;
        if record.state == JobState::Running {
            return Err(CodingError::RewindWhileRunning);
        }
        let verified = self.rewind_checkout(job).await?;
        let baseline = self.stored_baseline(job, &record)?;
        let records = self.state_root.join(job.as_str());
        let scope = AssignedPaths::parse(self.rewind_request(job)?.write_paths)
            .map_err(CodingError::write_paths)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let changes = job_changes(
            &self.artifacts,
            &records,
            &verified,
            &ignored,
            &CancellationToken::new(),
        )?;
        let foreign = if force {
            Vec::new()
        } else {
            self.foreign_paths(job, &verified, &changes)?
        };
        let current: Vec<String> = changes.into_iter().map(|change| change.path).collect();
        let report = if self.is_in_place() {
            let store = BeforeImageStore::open(&records).map_err(CodingError::owner)?;
            restore_before_images(
                &store,
                Path::new(verified.path().as_str()),
                &scope,
                &foreign,
            )?
        } else {
            let captured = self
                .artifacts
                .read_snapshot(baseline.snapshot_id())
                .map_err(CodingError::owner)?;
            restore_baseline(
                &self.artifacts,
                &captured,
                &PathBuf::from(verified.path().as_str()),
                &scope,
                &current,
                &foreign,
            )?
        };
        Ok(json!({
            "job_id": job,
            "baseline_snapshot": baseline.snapshot_id(),
            "restored_paths": report.restored,
            "removed_paths": report.removed,
            "out_of_scope_paths": report.out_of_scope,
            "foreign_paths": report.foreign,
            "not_restorable_paths": report.not_restorable,
            "authority": "none",
            "note": "the baseline is the state before the job's first turn: an in-place job restores its recorded before-images, a managed job its captured snapshot. A baseline restore changes file content only, never a command, download or database effect; shell-made changes are reported, never rewound",
        }))
    }

    /// The current changes the job's own journal does not account for. A path
    /// with no journal basis is never called foreign.
    fn foreign_paths(
        &self,
        job: &JobId,
        verified: &VerifiedCheckout,
        changes: &[RootChange],
    ) -> Result<Vec<String>, CodingError> {
        let Some(known) = super::super::known_commit::read(
            &self.state_root.join(job.as_str()),
            verified.checkout_id(),
        )?
        else {
            return Ok(Vec::new());
        };
        Ok(changes
            .iter()
            .filter(|change| {
                super::super::known_commit::is_foreign(
                    Some(&known),
                    &change.path,
                    change.kind,
                    change.content_sha256.as_ref(),
                )
            })
            .map(|change| change.path.clone())
            .collect())
    }

    /// The job's recorded working checkout, re-verified as its own at use time.
    /// An in-place job's root is its own folder, verified from the private
    /// baseline record it was fixed from; there is no managed checkout.
    async fn rewind_checkout(&self, job: &JobId) -> Result<VerifiedCheckout, CodingError> {
        if self.is_in_place() {
            let record_path = self.state_root.join(job.as_str()).join("inplace-root.json");
            if !record_path.exists() {
                return Err(CodingError::RewindNoCheckout);
            }
            let record = super::super::root_checkout::InPlaceRootRecord::read(&record_path)?;
            return verify_in_place_root(&InPlaceRootInput {
                artifacts: &self.artifacts,
                state_root: &self.state_root,
                root_id: record.root_id(),
                root_path: Path::new(record.root_path()),
                snapshot_id: record.snapshot_id(),
                plan_sha256: record.plan_sha256(),
            })
            .map_err(CodingError::owner);
        }
        let Some(checkout_id) = read_root_checkout(&self.state_root.join(job.as_str()))? else {
            return Err(CodingError::RewindNoCheckout);
        };
        verify_root_checkout(
            &OwnershipInput {
                store: &self.store,
                service: &self.service,
                repository: self.git_repository("rewind")?,
                artifacts: &self.artifacts,
                job_id: job,
                checkout_id: &checkout_id,
            },
            &CancellationToken::new(),
        )
        .await
        .map_err(CodingError::owner)
    }

    /// The job's persisted request, read for its write scope. Unlike a resume
    /// this asks the operator nothing: a rewind only ever narrows to the scope
    /// the job already stated.
    fn rewind_request(&self, job: &JobId) -> Result<CodingJobRequest, CodingError> {
        let bytes = context::read(
            &self.state_root.join(job.as_str()).join("request.json"),
            64 * 1024,
        )?;
        let request: CodingJobRequest =
            serde_json::from_slice(&bytes).map_err(CodingError::owner)?;
        if request.job_id() != job {
            return Err(CodingError::NotOwned);
        }
        Ok(request)
    }
}

/// The checkpoint turn a `rewind` command names, whichever of the two
/// spellings stated it: `--to <turn>` on the CLI, the positional `<turn>` in a
/// session. `None` is a listing, and stating both is refused rather than
/// resolved by picking one — two spellings that disagree name two different
/// requests. Both adapters resolve the turn here, so they cannot drift.
pub(crate) fn target(to: Option<u32>, turn: Option<u32>) -> Result<Option<u32>, CodingError> {
    match (to, turn) {
        (Some(_), Some(_)) => Err(CodingError::InvalidRequest),
        (Some(to), None) => Ok(Some(to)),
        (None, Some(turn)) => Ok(Some(turn)),
        (None, None) => Ok(None),
    }
}

/// The listing both surfaces render: every checkpoint, oldest first, with what
/// it holds and the bound the job keeps.
fn listing(store: &RewindStore, job: &JobId) -> Result<Value, CodingError> {
    let checkpoints = store.list()?;
    let missed = store.capture_failure();
    Ok(json!({
        "job_id": job,
        "checkpoints": checkpoints.iter().map(|checkpoint| json!({
            "turn": checkpoint.turn,
            "created_unix_ms": checkpoint.created_unix_ms,
            "paths": checkpoint.entries.iter().map(|entry| json!({
                "path": entry.path,
                "sha256": entry.sha256,
                "size": entry.size,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "maximum_checkpoints": super::MAX_REWIND_CHECKPOINTS,
        // A turn that could not be checkpointed is stated, never left as a
        // silent gap in the numbers.
        "not_checkpointed": missed.map(|(turn, class)| json!({"turn": turn, "class": class})),
        "authority": "none",
        "note": "a checkpoint is the working checkout's changed-file state at one turn boundary; `saya job rewind <job> --to <turn>` restores it",
    }))
}
