//! In-place review and finalize: the user's folder is the checkout.
//!
//! Review binds the job's exact changed paths — its own tool edits from the
//! before-image store, plus the shell-made changes Git status reports — and
//! writes the same bounded `review-<id>.jsonl` a managed review writes. Apply
//! writes nothing to the folder: it verifies the review is still current and
//! records a private `finalize.json` acknowledging that exact state, so a
//! second apply of the same review reports `already_applied` instead of
//! finalizing twice.

use super::{
    CodingError, CodingReview, NativeCodingJob, RootChange, context,
    in_place_changes::{self, in_place_tool_changes},
    in_place_start,
    review_diff::BaselineSource,
    rewind::{RewindStore, store_root},
};
use saya_harness::coding::{
    integration::{DeltaKind, EvidenceBinding},
    local::{
        JobBaseline, LocalBaseline,
        before_image::BeforeImageStore,
        shell_changes::{
            MAX_SHELL_HASH_FILE_BYTES, MAX_SHELL_HASH_FILES, ShellChangeKind, ShellChanges,
            shell_changes as read_shell_changes,
        },
    },
    snapshot::MAX_SNAPSHOT_MANIFEST_BYTES,
    worktree::VerifiedCheckout,
};
use saya_harness::workspace::Workspace;
use saya_types::{ManagedCheckoutId, Sha256Digest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::time::UNIX_EPOCH;

const FINALIZE_FILE: &str = "finalize.json";
pub(super) const MAX_FINALIZE_BYTES: usize = 16 * 1024;
const MAX_FINALIZE_PATHS: usize = 512;
const MAX_REVIEW_BYTES: usize = 1024 * 1024;

/// The private acknowledgement one in-place apply writes. It is identity and
/// provenance only: the folder's bytes are never touched by apply.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FinalizeRecord {
    version: u32,
    mode: String,
    source_sha256: Sha256Digest,
    finalized_unix_ms: i64,
    checkpoint_turn: Option<u32>,
    changed_paths: Vec<String>,
}

fn read_finalize(path: &Path) -> Result<Option<FinalizeRecord>, CodingError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = context::read(path, MAX_FINALIZE_BYTES)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(CodingError::owner)
}

/// One in-place review's evidence: the binding and the preview it describes.
struct InPlaceEvidence {
    store: BeforeImageStore,
    changes: Vec<RootChange>,
    shell: ShellChanges,
    files: Vec<(String, Option<Sha256Digest>)>,
}

impl NativeCodingJob {
    /// The job's fixed in-place baseline, authenticated by the record's plan
    /// digest. A stored Git plan here is a shape error, never a fallback.
    pub(super) fn in_place_baseline(&self) -> Result<LocalBaseline, CodingError> {
        let record = self.in_place_record()?;
        let bytes = context::read(
            &self.records.join("baseline.json"),
            MAX_SNAPSHOT_MANIFEST_BYTES,
        )?;
        match JobBaseline::from_verified_bytes(&bytes, record.plan_sha256())
            .map_err(CodingError::owner)?
        {
            JobBaseline::InPlace(baseline) => Ok(baseline),
            JobBaseline::Git(_) => Err(CodingError::NotOwned),
        }
    }

    /// The acknowledgement the last in-place apply wrote, if any: its review
    /// source digest and the checkpoint turn it recorded.
    pub(super) fn in_place_finalize(
        &self,
    ) -> Result<Option<(Sha256Digest, Option<u32>)>, CodingError> {
        Ok(read_finalize(&self.records.join(FINALIZE_FILE))?
            .map(|record| (record.source_sha256, record.checkpoint_turn)))
    }

    /// The job's own tool edits and the shell-made changes beside them, with
    /// the exact path list the review binds.
    async fn in_place_evidence(
        &self,
        verified: &VerifiedCheckout,
    ) -> Result<InPlaceEvidence, CodingError> {
        let store = BeforeImageStore::open(&self.records).map_err(CodingError::owner)?;
        let workspace =
            Workspace::open(Path::new(verified.path().as_str())).map_err(CodingError::owner)?;
        let changes = in_place_tool_changes(&store, &workspace)?;
        let touched: HashSet<String> = changes.iter().map(|change| change.path.clone()).collect();
        let start = in_place_start::read(&self.records);
        let shell = read_shell_changes(
            &self.application.service,
            Path::new(verified.path().as_str()),
            &start,
            &touched,
            &self.cancellation,
        )
        .await;
        let files = evidence_paths(verified, &workspace, &changes, &shell)?;
        Ok(InPlaceEvidence {
            store,
            changes,
            shell,
            files,
        })
    }

    pub(super) async fn review_in_place(
        &self,
        checkout: &ManagedCheckoutId,
    ) -> Result<CodingReview, CodingError> {
        let verified = self.in_place_checkout()?;
        if verified.checkout_id() != checkout {
            return Err(CodingError::NotOwned);
        }
        let baseline = self.in_place_baseline()?;
        let evidence = self.in_place_evidence(&verified).await?;
        let binding =
            EvidenceBinding::bind_paths(&verified, &evidence.files).map_err(CodingError::owner)?;
        let checkpoints = RewindStore::open(&store_root(&self.records)).turns()?.len();
        let mut preview = json!({
            "mode": "in_place",
            "root": {
                "path": baseline.root_path(),
                "root_id": baseline.repository_id(),
                "git": self.in_place_record()?.git(),
            },
            "baseline": {
                "snapshot_id": baseline.snapshot_id(),
                "kind": "edit_time",
            },
            "recorded": in_place_changes::recorded_facts(&evidence.changes, evidence.store.overflowed()),
            "candidate": checkout,
            "candidate_path": verified.path(),
            "source_sha256": binding.source_sha256(),
            "changes": evidence.changes.iter().map(|entry| json!({"path":entry.path,"kind":format!("{:?}",entry.kind),"bytes":entry.bytes})).collect::<Vec<_>>(),
            "shell_changes": serde_json::to_value(&evidence.shell).map_err(CodingError::owner)?,
            "diff": self.review_diff(&verified, BaselineSource::BeforeImages(&evidence.store), &evidence.changes)?,
            "applied_in_place": true,
            "apply_requires_fresh_approval": false,
            "undo": {"checkpoints": checkpoints, "baseline_restore": true},
            "note": "files only; shell-made changes are reported, never rewound",
        });
        if let Some(foreign) = self.foreign_changes(&verified, &evidence.changes)? {
            preview["foreign_changes"] = json!(foreign);
        }
        context::append_line(
            &self
                .records
                .join(format!("review-{}.jsonl", checkout.as_str())),
            &serde_json::to_vec(&preview).map_err(CodingError::owner)?,
            MAX_REVIEW_BYTES,
        )?;
        Ok(CodingReview {
            job: self.job_id().clone(),
            epoch: self.epoch,
            binding,
            preview,
        })
    }

    /// Applies one in-place review: verify the job's changed paths are still
    /// exactly what was reviewed, then record the acknowledgement. Nothing is
    /// copied and no file in the folder is written.
    pub(super) async fn apply_in_place(&self, review: &CodingReview) -> Result<Value, CodingError> {
        if review.job != *self.job_id() || review.epoch != self.epoch {
            return Err(CodingError::StaleReview);
        }
        let verified = self.in_place_checkout()?;
        if review.binding.candidate_checkout_id() != verified.checkout_id() {
            return Err(CodingError::StaleReview);
        }
        let evidence = self.in_place_evidence(&verified).await?;
        review
            .binding
            .verify_paths(&evidence.files)
            .map_err(|_| CodingError::StaleReview)?;
        let path = self.records.join(FINALIZE_FILE);
        let already_applied = read_finalize(&path)?.is_some_and(|record| {
            record.mode == "in_place" && record.source_sha256 == *review.source_sha256()
        });
        if !already_applied {
            let changed_paths = evidence
                .changes
                .iter()
                .take(MAX_FINALIZE_PATHS)
                .map(|change| change.path.clone())
                .collect();
            let checkpoint_turn = RewindStore::open(&store_root(&self.records))
                .turns()?
                .last()
                .copied();
            let record = FinalizeRecord {
                version: 1,
                mode: "in_place".to_owned(),
                source_sha256: review.source_sha256().clone(),
                finalized_unix_ms: crate::interactive::session::continuation::now_unix_ms(),
                checkpoint_turn,
                changed_paths,
            };
            let bytes = serde_json::to_vec(&record).map_err(CodingError::owner)?;
            context::write_replace(&path, &bytes, MAX_FINALIZE_BYTES)?;
        }
        Ok(json!({
            "applied": true,
            "mode": "in_place",
            "already_applied": already_applied,
            "source_sha256": review.source_sha256(),
        }))
    }

    /// The changed paths the job's own committed-byte journal does not account
    /// for, or `None` when no journal exists (so review states no basis it does
    /// not have). A removal is the job's own only when the journal records it.
    fn foreign_changes(
        &self,
        verified: &VerifiedCheckout,
        changes: &[RootChange],
    ) -> Result<Option<Vec<String>>, CodingError> {
        let Some(known) = super::known_commit::read(&self.records, verified.checkout_id())? else {
            return Ok(None);
        };
        let foreign = changes
            .iter()
            .filter(|change| {
                super::known_commit::is_foreign(
                    Some(&known),
                    &change.path,
                    change.kind,
                    change.content_sha256.as_ref(),
                )
            })
            .map(|change| change.path.clone())
            .collect();
        Ok(Some(foreign))
    }
}

/// The exact path list a review binds: every tool-changed path with the digest
/// it holds now (or `absent` for a removal), plus every shell-changed path.
/// Shell files are hashed up to the stated count and byte bound; beyond it, and
/// for any path this workspace refuses to read, the digest folds in size and
/// mtime, so a later edit still invalidates the review.
fn evidence_paths(
    verified: &VerifiedCheckout,
    workspace: &Workspace,
    changes: &[RootChange],
    shell: &ShellChanges,
) -> Result<Vec<(String, Option<Sha256Digest>)>, CodingError> {
    let root = Path::new(verified.path().as_str());
    let mut files = Vec::with_capacity(changes.len());
    for change in changes {
        let digest = match (&change.content_sha256, change.kind) {
            (Some(digest), _) => Some(digest.clone()),
            (None, DeltaKind::Deleted) => None,
            (None, _) => Some(pseudo_digest(root, &change.path)?),
        };
        files.push((change.path.clone(), digest));
    }
    let ShellChanges::Recorded { paths, .. } = shell else {
        return Ok(files);
    };
    let mut hashed = 0usize;
    for change in paths {
        if change.kind == ShellChangeKind::Deleted {
            files.push((change.path.clone(), None));
            continue;
        }
        let digest = if hashed < MAX_SHELL_HASH_FILES {
            match workspace.read(&change.path, MAX_SHELL_HASH_FILE_BYTES) {
                Ok(file) => {
                    hashed += 1;
                    Sha256Digest::parse(&file.digest).map_err(CodingError::owner)?
                }
                Err(_) => pseudo_digest(root, &change.path)?,
            }
        } else {
            pseudo_digest(root, &change.path)?
        };
        files.push((change.path.clone(), Some(digest)));
    }
    Ok(files)
}

/// A digest over a path's size and mtime for a file this workspace will not
/// read. It is not a content digest; it is enough to notice a later edit.
fn pseudo_digest(root: &Path, path: &str) -> Result<Sha256Digest, CodingError> {
    let (size, mtime_ns) = match std::fs::symlink_metadata(root.join(path)) {
        Ok(metadata) => (
            metadata.len(),
            metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX))
                .unwrap_or(0),
        ),
        Err(_) => (0, 0),
    };
    super::create::digest(format!("unreadable:{path}:{size}:{mtime_ns}").as_bytes())
}
