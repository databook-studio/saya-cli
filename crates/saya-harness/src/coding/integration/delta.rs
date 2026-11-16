//! Actual checkout delta between a verified managed checkout and its captured baseline.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use saya_agent::CancellationToken;
use saya_types::{Sha256Digest, SnapshotId};

use crate::coding::git::IgnoredPaths;
use crate::coding::snapshot::{CapturedEntry, CapturedSnapshot};
use crate::coding::worktree::VerifiedCheckout;

use super::delta_types::{
    CheckoutDelta, DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, LiveFile, NotIncludedPath,
    NotIncludedReason, PathDelta, WalkOutcome,
};
use super::delta_walk::walk_checkout_with_root_git_directory;
use super::paths::casefold_collision;

/// Classifies the live checkout against the verified captured baseline.
///
/// The delta is derived only from the [`VerifiedCheckout`], the verified
/// baseline snapshot, the checkout's Git ignore verdicts, and the live
/// filesystem — never from a child's claim. Ignored paths are not job content
/// and never appear; a hidden file the baseline did not hold counts as
/// [`DeltaKind::Added`]. A path the walk cannot represent is reported as not
/// included, and a
/// bound trip returns the included prefix with `changes_truncated` set, so a
/// review and apply still cover what was included. Renames are not paired
/// here: a rename is a [`DeltaKind::Deleted`] plus a [`DeltaKind::Added`], and
/// ownership of each endpoint is judged separately.
///
/// # Errors
///
/// Returns [`DeltaError::SnapshotMismatch`] when the snapshot identity or
/// captured-manifest digest disagrees with the verified checkout. Cancellation,
/// I/O failures, and a case-fold collision across the union of baseline and
/// live paths are refused; ordinary bounds are reported in the delta instead.
pub fn compute_delta(
    verified: &VerifiedCheckout,
    snapshot: &CapturedSnapshot,
    ignored: &IgnoredPaths,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<CheckoutDelta, DeltaError> {
    compute_delta_at_root(
        Path::new(verified.path().as_str()),
        verified.snapshot_id(),
        verified.snapshot_sha256(),
        snapshot,
        cancellation,
        deadline,
        WalkPolicy {
            ignored,
            strict: false,
        },
    )
}

/// Classifies the live folder at `root` against one captured baseline. The
/// caller supplies the trusted `(snapshot_id, captured_manifest_sha256)`
/// binding; a baseline that disagrees is refused before the walk. The
/// root-level `.saya` directory is bounded but never classified, so scratch
/// and review output never appear as additions.
pub fn compute_delta_at(
    root: &Path,
    snapshot_id: &SnapshotId,
    snapshot_sha256: &Sha256Digest,
    baseline: &CapturedSnapshot,
    cancellation: &CancellationToken,
    deadline: Duration,
) -> Result<CheckoutDelta, DeltaError> {
    let ignored = IgnoredPaths::none();
    compute_delta_at_root(
        root,
        snapshot_id,
        snapshot_sha256,
        baseline,
        cancellation,
        deadline,
        WalkPolicy {
            ignored: &ignored,
            strict: true,
        },
    )
}

/// Walk policy shared by the two entry points: the checkout's ignore verdicts
/// and whether an unrepresentable path fails the delta.
struct WalkPolicy<'a> {
    ignored: &'a IgnoredPaths,
    /// The raw entry point keeps its original refusal: a path the walk cannot
    /// represent fails the delta instead of being reported.
    strict: bool,
}

fn compute_delta_at_root(
    root: &Path,
    snapshot_id: &SnapshotId,
    snapshot_sha256: &Sha256Digest,
    baseline: &CapturedSnapshot,
    cancellation: &CancellationToken,
    deadline: Duration,
    policy: WalkPolicy<'_>,
) -> Result<CheckoutDelta, DeltaError> {
    if baseline.snapshot_id() != snapshot_id
        || baseline.captured_manifest_sha256() != snapshot_sha256
    {
        return Err(DeltaError::SnapshotMismatch);
    }
    let now = Instant::now();
    let deadline_at = now
        .checked_add(deadline)
        .unwrap_or_else(|| now + DEFAULT_DELTA_DEADLINE);
    let WalkOutcome {
        files: live,
        not_included,
        not_included_overflow,
        aborted,
    } = walk_checkout_with_root_git_directory(
        root,
        cancellation,
        deadline_at,
        false,
        &HashSet::new(),
        policy.ignored,
    )?;
    if policy.strict
        && let Some(error) = strict_error(&not_included, not_included_overflow, aborted)
    {
        return Err(error);
    }
    if casefold_collision(
        baseline
            .entries()
            .iter()
            .map(|entry| entry.path().as_str())
            .chain(live.iter().map(|file| file.path.as_str())),
    ) {
        return Err(DeltaError::CaseFoldCollision);
    }
    let baseline_entries: HashMap<&str, &CapturedEntry> = baseline
        .entries()
        .iter()
        .map(|entry| (entry.path().as_str(), entry))
        .collect();
    let live_paths: HashSet<String> = live.iter().map(|file| file.path.clone()).collect();
    let mut deltas = Vec::new();
    for LiveFile {
        path,
        mode,
        bytes,
        sha256,
    } in live
    {
        match baseline_entries.get(path.as_str()) {
            None => deltas.push(PathDelta::new(
                path,
                DeltaKind::Added,
                mode,
                bytes,
                Some(sha256),
            )),
            Some(entry) => {
                if entry.content_sha256() != &sha256 || entry.mode() != mode {
                    deltas.push(PathDelta::new(
                        path,
                        DeltaKind::Modified,
                        mode,
                        bytes,
                        Some(sha256),
                    ));
                }
            }
        }
    }
    // A partial walk or an omitted path cannot prove a baseline path absent,
    // so deletions are classified only from a complete, fully named walk.
    if aborted.is_none() && not_included_overflow == 0 {
        for entry in baseline.entries() {
            let path = entry.path().as_str();
            if live_paths.contains(path)
                || policy.ignored.covers(Path::new(path))
                || not_included_covers(&not_included, path)
            {
                continue;
            }
            deltas.push(PathDelta::new(
                path.to_owned(),
                DeltaKind::Deleted,
                entry.mode(),
                0,
                None,
            ));
        }
    }
    let changes_truncated =
        aborted.is_some() || !not_included.is_empty() || not_included_overflow > 0;
    Ok(CheckoutDelta::reported(
        deltas,
        not_included,
        not_included_overflow,
        changes_truncated,
    ))
}

/// The original refusal for the strict entry points: the first bound trip or
/// unrepresentable path, in the walk's order.
fn strict_error(
    not_included: &[NotIncludedPath],
    not_included_overflow: usize,
    aborted: Option<DeltaError>,
) -> Option<DeltaError> {
    if let Some(error) = aborted {
        return Some(error);
    }
    if not_included.is_empty() && not_included_overflow == 0 {
        return None;
    }
    let reason = match not_included.first() {
        Some(item) => item.reason(),
        None => return Some(DeltaError::TooManyFiles),
    };
    Some(match reason {
        NotIncludedReason::NestedRepository => DeltaError::NestedRepository,
        NotIncludedReason::SpecialFile => DeltaError::Symlink,
        NotIncludedReason::UnsafePath => DeltaError::UnsafePath,
        NotIncludedReason::Oversize => DeltaError::FileTooLarge,
    })
}

/// Whether `path`, or an ancestor directory, is a path the walk reported as
/// not included.
fn not_included_covers(paths: &[NotIncludedPath], path: &str) -> bool {
    let mut candidate = Path::new(path);
    loop {
        if paths.iter().any(|item| Path::new(item.path()) == candidate) {
            return true;
        }
        match candidate.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => candidate = parent,
            _ => return false,
        }
    }
}
