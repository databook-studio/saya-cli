//! Actual checkout delta between a verified managed checkout and its captured baseline.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use saya_agent::CancellationToken;

use crate::coding::snapshot::{CapturedEntry, CapturedSnapshot, SnapshotFileMode};
use crate::coding::worktree::VerifiedCheckout;

use super::delta_types::{
    CheckoutDelta, DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, LiveFile, PathDelta,
};
use super::delta_walk::walk_checkout;
use super::paths::casefold_collision;

/// Classifies the live checkout against the verified captured baseline.
///
/// The delta is derived only from the [`VerifiedCheckout`], the verified baseline
/// snapshot, and the live filesystem — never from a child's claim. Hidden,
/// ignored, and omitted-private files count as [`DeltaKind::Added`] because the
/// content-only baseline cannot tell them apart. A bound trip is an error
/// (rejection), never a truncated delta. Renames are not paired here: a rename is
/// a [`DeltaKind::Deleted`] plus a [`DeltaKind::Added`], and ownership of each
/// endpoint is judged separately.
///
/// # Errors
///
/// Returns [`DeltaError::SnapshotMismatch`] when the snapshot identity or
/// captured-manifest digest disagrees with the verified checkout. Otherwise the
/// walk's typed rejections (`Cancelled`, `DeadlineExceeded`, `Symlink`,
/// `NestedRepository`, `UnsafePath`, `Io`, and the size and file-count bounds)
/// propagate verbatim, and a case-fold collision across the union of baseline and
/// live paths is [`DeltaError::CaseFoldCollision`].
pub fn compute_delta(
    verified: &VerifiedCheckout,
    snapshot: &CapturedSnapshot,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<CheckoutDelta, DeltaError> {
    if snapshot.snapshot_id() != verified.snapshot_id()
        || snapshot.captured_manifest_sha256() != verified.snapshot_sha256()
    {
        return Err(DeltaError::SnapshotMismatch);
    }
    let now = Instant::now();
    let deadline_at = now
        .checked_add(deadline)
        .unwrap_or_else(|| now + DEFAULT_DELTA_DEADLINE);
    let live = walk_checkout(
        Path::new(verified.path().as_str()),
        cancellation,
        deadline_at,
    )?;
    if casefold_collision(
        snapshot
            .entries()
            .iter()
            .map(|entry| entry.path().as_str())
            .chain(live.iter().map(|file| file.path.as_str())),
    ) {
        return Err(DeltaError::CaseFoldCollision);
    }
    let baseline: HashMap<&str, &CapturedEntry> = snapshot
        .entries()
        .iter()
        .map(|entry| (entry.path().as_str(), entry))
        .collect();
    let live_paths: HashSet<String> = live.iter().map(|file| file.path.clone()).collect();
    let mut deltas = Vec::new();
    for LiveFile {
        path,
        executable,
        bytes,
        sha256,
    } in live
    {
        match baseline.get(path.as_str()) {
            None => deltas.push(PathDelta::new(path, DeltaKind::Added, bytes, Some(sha256))),
            Some(entry) => {
                if entry.content_sha256() != &sha256
                    || (entry.mode() == SnapshotFileMode::Executable) != executable
                {
                    deltas.push(PathDelta::new(
                        path,
                        DeltaKind::Modified,
                        bytes,
                        Some(sha256),
                    ));
                }
            }
        }
    }
    for entry in snapshot.entries() {
        let path = entry.path().as_str();
        if !live_paths.contains(path) {
            deltas.push(PathDelta::new(path.to_owned(), DeltaKind::Deleted, 0, None));
        }
    }
    Ok(CheckoutDelta::new(deltas))
}
