//! The model-facing rewind: list this run's checkpoints, or restore the
//! working checkout to one while the run is live.
//!
//! This is the same store and the same [`restore`](super::apply::restore) the
//! operator rewind uses — there is no second restore path. What differs is the
//! surface: the model names a checkpoint id from a bounded listing (or `last`),
//! may restrict the restore to exact workspace-relative paths, and receives
//! the digest each restored path holds so a following `edit` needs no
//! extra read. A rewind is files only: it never undoes a command, a download or
//! a database effect.

use super::apply::{Before, restore};
use super::{MAX_REWIND_CHECKPOINTS, MAX_REWIND_FILES_PER_CHECKPOINT, RestoreReport, RewindStore};
use crate::commands::coding::{CodingError, NativeCodingJob, job_changes};
use saya_harness::coding::integration::{AssignedPaths, MAX_ASSIGNED_PATH_BYTES};
use saya_harness::coding::local::before_image::BeforeImageStore;
use saya_types::WorkspacePath;
use serde_json::{Value, json};
use std::path::Path;

/// The most checkpoints one listing returns: the newest, since those are the
/// states a run can still return to.
const MAX_LISTED_CHECKPOINTS: usize = 20;

/// The most path names one listed checkpoint summarises.
const MAX_LISTED_PATHS: usize = 8;

/// The most changed-path entries one restore result returns. The arrays are
/// trimmed together, and `omitted_paths` states how many were not listed.
const MAX_RESULT_PATHS: usize = 64;

impl NativeCodingJob {
    /// Lists this run's checkpoints, or restores its working checkout to one.
    ///
    /// No `checkpoint` is a listing. `checkpoint` is a listed id or `last`;
    /// `paths`, when stated, restricts the restore to those exact
    /// workspace-relative files. Every path is one the job's own write scope
    /// already admits; a checkpoint path outside it is never touched and is
    /// reported.
    pub(crate) async fn model_rewind(
        &self,
        checkpoint: Option<&str>,
        paths: Option<&[String]>,
    ) -> Result<Value, CodingError> {
        let store = self.rewind_store();
        let Some(checkpoint) = checkpoint else {
            return listing(&store);
        };
        let only = requested_paths(paths)?;
        let turn = checkpoint_turn(&store, checkpoint)?;
        let Some(verified) = self.verified_root_checkout().await? else {
            return Err(CodingError::RewindNoCheckout);
        };
        let scope = AssignedPaths::parse(self.request.write_paths.clone())
            .map_err(CodingError::write_paths)?;
        let ignored = self.checkout_ignored_paths(&verified).await?;
        let changes = job_changes(
            &self.application.artifacts,
            &self.records,
            &verified,
            &ignored,
            &self.cancellation,
        )?;
        let foreign = match super::super::known_commit::read(&self.records, verified.checkout_id())?
        {
            Some(known) => changes
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
                .collect(),
            None => Vec::new(),
        };
        let before = self
            .application
            .is_in_place()
            .then(|| BeforeImageStore::open(&self.records))
            .transpose()
            .map_err(CodingError::owner)?;
        let current: Vec<String> = changes.into_iter().map(|change| change.path).collect();
        let report = restore(
            &store,
            Path::new(verified.path().as_str()),
            &scope,
            turn,
            &current,
            only.as_deref(),
            Before {
                images: before.as_ref(),
                foreign: &foreign,
            },
        )?;
        store.record_rewind(
            turn,
            crate::interactive::session::continuation::now_unix_ms(),
        )?;
        Ok(result(turn, report))
    }
}

/// The listing the model reads: newest first, bounded, with a short summary of
/// each checkpoint's changed paths and the ids it may name.
fn listing(store: &RewindStore) -> Result<Value, CodingError> {
    let all = store.list()?;
    let shown: Vec<_> = all.iter().rev().take(MAX_LISTED_CHECKPOINTS).collect();
    Ok(json!({
        "checkpoints": shown.iter().map(|checkpoint| json!({
            "id": checkpoint.turn.to_string(),
            "turn": checkpoint.turn,
            "created_unix_ms": checkpoint.created_unix_ms,
            "paths": checkpoint.entries.iter()
                .take(MAX_LISTED_PATHS)
                .map(|entry| entry.path.clone())
                .collect::<Vec<_>>(),
            "path_count": checkpoint.entries.len(),
        })).collect::<Vec<_>>(),
        "omitted": all.len().saturating_sub(shown.len()),
        "maximum_checkpoints": MAX_REWIND_CHECKPOINTS,
        "not_checkpointed": store.capture_failure()
            .map(|(turn, class)| json!({"turn": turn, "class": class})),
        "note": "pass an `id` as `checkpoint` (or `last`); `paths` restricts the restore to those workspace-relative files",
    }))
}

/// The restore's result: the version each restored path holds, the paths
/// removed to match the checkpoint, and what the restore did not touch.
fn result(turn: u32, report: RestoreReport) -> Value {
    let RestoreReport {
        out_of_scope,
        foreign,
        not_restorable,
        changes,
        ..
    } = report;
    let mut restored = Vec::new();
    let mut removed = Vec::new();
    for change in changes {
        match change.sha256 {
            Some(sha256) => restored.push(json!({
                "path": change.path,
                "sha256": sha256,
                "size": change.size,
            })),
            None => removed.push(json!(change.path)),
        }
    }
    let total =
        restored.len() + removed.len() + out_of_scope.len() + foreign.len() + not_restorable.len();
    restored.truncate(MAX_RESULT_PATHS);
    removed.truncate(MAX_RESULT_PATHS.saturating_sub(restored.len()));
    let mut out_of_scope = out_of_scope;
    out_of_scope.truncate(MAX_RESULT_PATHS.saturating_sub(restored.len() + removed.len()));
    let mut foreign = foreign;
    foreign.truncate(
        MAX_RESULT_PATHS.saturating_sub(restored.len() + removed.len() + out_of_scope.len()),
    );
    let mut not_restorable = not_restorable;
    not_restorable.truncate(
        MAX_RESULT_PATHS
            .saturating_sub(restored.len() + removed.len() + out_of_scope.len() + foreign.len()),
    );
    let listed =
        restored.len() + removed.len() + out_of_scope.len() + foreign.len() + not_restorable.len();
    json!({
        "checkpoint": turn,
        "restored_paths": restored,
        "removed_paths": removed,
        "out_of_scope_paths": out_of_scope,
        "foreign_paths": foreign,
        "not_restorable_paths": not_restorable,
        "omitted_paths": total.saturating_sub(listed),
        "note": "files only: a rewind never undoes a command, download or database effect; shell-made changes are reported, never rewound",
    })
}

/// Resolves the checkpoint the model named: `last` is the newest, every other
/// spelling is a listed id.
fn checkpoint_turn(store: &RewindStore, checkpoint: &str) -> Result<u32, CodingError> {
    let checkpoints = store.list()?;
    if checkpoint == "last" {
        return checkpoints
            .last()
            .map(|checkpoint| checkpoint.turn)
            .ok_or(CodingError::RewindNoCheckpoint);
    }
    let turn: u32 = checkpoint
        .parse()
        .map_err(|_| CodingError::InvalidRequest)?;
    if !checkpoints.iter().any(|checkpoint| checkpoint.turn == turn) {
        return Err(CodingError::RewindUnknownTurn { turn });
    }
    Ok(turn)
}

/// Validates the optional path restriction: exact workspace-relative paths,
/// distinct, and within the same count bound one checkpoint holds.
fn requested_paths(paths: Option<&[String]>) -> Result<Option<Vec<String>>, CodingError> {
    let Some(paths) = paths else {
        return Ok(None);
    };
    if paths.is_empty() || paths.len() > MAX_REWIND_FILES_PER_CHECKPOINT {
        return Err(CodingError::InvalidRequest);
    }
    let mut seen = std::collections::HashSet::with_capacity(paths.len());
    let mut requested = Vec::with_capacity(paths.len());
    for path in paths {
        if path.len() > MAX_ASSIGNED_PATH_BYTES
            || WorkspacePath::try_from(path.clone()).is_err()
            || !seen.insert(path.clone())
        {
            return Err(CodingError::InvalidRequest);
        }
        requested.push(path.clone());
    }
    Ok(Some(requested))
}
