//! Applying accepted child deltas into the candidate working tree: pure
//! overlap detection for callers, and fail-closed application of one accepted
//! delta with every child byte re-verified at apply time. Every access starts
//! from the descriptor each workspace pinned at open, so a replaced directory
//! cannot redirect the chmod or the stat.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::Path;

use saya_agent::CancellationToken;

use crate::coding::snapshot::CapturedSnapshot;
use crate::coding::worktree::VerifiedCheckout;
use crate::workspace::{MAX_IO_BYTES, Workspace};

use super::{ChangeReceipt, CheckoutDelta, DeltaKind, PathDelta, ReceiptChange, ReceiptOutcome};

/// Why a delta could not be applied into the candidate working tree.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("the receipt is not within the assignment")]
    ReceiptRejected,
    #[error("the receipt does not describe this delta")]
    ReceiptMismatch,
    #[error("a child file no longer matches its receipt")]
    ContentMismatch,
    #[error("a file exceeds the apply size bound")]
    Oversize,
    #[error("a filesystem operation was refused")]
    Fs,
    #[error("the apply was cancelled")]
    Cancelled,
}

/// A checkout delta whose receipt has been proven to describe it exactly:
/// within assignment, same checkout, and one equal change per delta entry.
pub struct AcceptedDelta<'a> {
    verified: &'a VerifiedCheckout,
    delta: &'a CheckoutDelta,
}

impl<'a> AcceptedDelta<'a> {
    /// Fails closed unless the receipt describes the delta exactly.
    pub fn new(
        verified: &'a VerifiedCheckout,
        receipt: &ChangeReceipt,
        delta: &'a CheckoutDelta,
    ) -> Result<Self, ApplyError> {
        if receipt.outcome() != ReceiptOutcome::WithinAssignment {
            return Err(ApplyError::ReceiptRejected);
        }
        if receipt.checkout_id() != verified.checkout_id()
            || !receipt_describes_delta(receipt, delta)
        {
            return Err(ApplyError::ReceiptMismatch);
        }
        Ok(Self { verified, delta })
    }

    pub fn verified(&self) -> &VerifiedCheckout {
        self.verified
    }

    pub fn delta(&self) -> &CheckoutDelta {
        self.delta
    }
}

/// Whether the receipt lists one equal change per delta entry, in order.
fn receipt_describes_delta(receipt: &ChangeReceipt, delta: &CheckoutDelta) -> bool {
    receipt.changes().len() == delta.len()
        && receipt
            .changes()
            .iter()
            .zip(delta.entries())
            .all(pair_matches)
}

fn pair_matches(change: &ReceiptChange, entry: &PathDelta) -> bool {
    change.path() == entry.path()
        && change.kind() == entry.kind()
        && change.bytes() == entry.bytes()
        && change.content_sha256() == entry.content_sha256()
}

/// One conflicting pair; [`std::fmt::Debug`] prints the indexes only.
pub struct OverlapPair {
    first: usize,
    second: usize,
    path: String,
}

impl OverlapPair {
    pub fn first(&self) -> usize {
        self.first
    }

    pub fn second(&self) -> usize {
        self.second
    }

    /// The conflicting path: the longer of the two touched paths.
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Debug for OverlapPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlapPair")
            .field("first", &self.first)
            .field("second", &self.second)
            .finish()
    }
}

/// The deduplicated conflicts among accepted deltas.
pub struct OverlapConflict {
    pairs: Vec<OverlapPair>,
}

impl OverlapConflict {
    /// The conflicting pairs, in first-detected order.
    pub fn pairs(&self) -> &[OverlapPair] {
        &self.pairs
    }
}

impl fmt::Debug for OverlapConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlapConflict")
            .field("pair_count", &self.pairs.len())
            .finish()
    }
}

/// Detects whether any two accepted deltas touch conflicting paths: the same
/// path, or a path and one of its directory ancestors at a component
/// boundary. Pure: no filesystem access; the caller detects before applying.
pub fn detect_overlaps(accepted: &[AcceptedDelta<'_>]) -> Result<(), OverlapConflict> {
    let mut touches: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, accepted_delta) in accepted.iter().enumerate() {
        for entry in accepted_delta.delta().entries() {
            touches.entry(entry.path()).or_default().push(index);
        }
    }

    let mut seen: HashSet<(usize, usize, &str)> = HashSet::new();
    let mut pairs: Vec<OverlapPair> = Vec::new();
    for (index, accepted_delta) in accepted.iter().enumerate() {
        for entry in accepted_delta.delta().entries() {
            let path = entry.path();
            let ancestors = path.match_indices('/').map(|(slash, _)| &path[..slash]);
            for touched in std::iter::once(path).chain(ancestors) {
                if let Some(indexes) = touches.get(touched) {
                    for &other in indexes {
                        if other == index {
                            continue;
                        }
                        let (first, second) = (index.min(other), index.max(other));
                        if seen.insert((first, second, path)) {
                            pairs.push(OverlapPair {
                                first,
                                second,
                                path: path.to_owned(),
                            });
                        }
                    }
                }
            }
        }
    }
    if pairs.is_empty() {
        Ok(())
    } else {
        Err(OverlapConflict { pairs })
    }
}

/// Applies one already-accepted delta into the candidate working tree,
/// re-reading and re-verifying every child byte against the receipt digest at
/// apply time; a drifted child fails closed and files over the 8 MiB write
/// bound are refused whole. Every access starts from the descriptor each
/// workspace pinned at open — the candidate and the verified checkout — so a
/// replaced directory cannot redirect the chmod or the stat. Overlaps are the
/// caller's to detect before the first apply and are never resolved here; an
/// interrupted candidate is discarded and rebuilt, never resumed.
pub fn apply_delta(
    candidate_root: &Path,
    accepted: &AcceptedDelta<'_>,
    baseline: &CapturedSnapshot,
    cancellation: &CancellationToken,
) -> Result<(), ApplyError> {
    let workspace = Workspace::open(candidate_root).map_err(|_| ApplyError::Fs)?;
    let child_workspace = Workspace::open(Path::new(accepted.verified().path().as_str()))
        .map_err(|_| ApplyError::Fs)?;
    for entry in accepted.delta().entries() {
        if cancellation.is_cancelled() {
            return Err(ApplyError::Cancelled);
        }
        apply_entry(&workspace, &child_workspace, entry, baseline)?;
    }
    Ok(())
}

/// Applies one delta entry into `workspace`, reading the child bytes from
/// `child_workspace` and re-verifying them against the receipt digest at
/// apply time; a drifted child fails closed and files over the 8 MiB write
/// bound are refused whole. Every access starts from the descriptor each
/// workspace pinned at open, so a replaced directory cannot redirect the
/// chmod or the stat. Deletions skip the mode fix-up: the entry is removed.
pub(super) fn apply_entry(
    workspace: &Workspace,
    child_workspace: &Workspace,
    entry: &PathDelta,
    baseline: &CapturedSnapshot,
) -> Result<(), ApplyError> {
    apply_entry_with_mode(workspace, child_workspace, entry, baseline, None)
}

/// Apply-back supplies the mode from its durable whole-operation observation.
pub(super) fn apply_entry_with_mode(
    workspace: &Workspace,
    child_workspace: &Workspace,
    entry: &PathDelta,
    baseline: &CapturedSnapshot,
    mode: Option<u32>,
) -> Result<(), ApplyError> {
    if entry.kind() == DeltaKind::Deleted {
        let baseline_hex = baseline_hex(baseline, entry.path())?;
        workspace
            .remove_guarded(entry.path(), baseline_hex)
            .map_err(|_| ApplyError::Fs)?;
        return Ok(());
    }
    if entry.bytes() > MAX_IO_BYTES as u64 {
        return Err(ApplyError::Oversize);
    }
    let file = super::apply_fs::read_child_file(child_workspace, entry.path())?;
    if Some(file.sha256_hex.as_str()) != entry.content_sha256().map(|d| d.as_str())
        || mode.is_some_and(|expected| (expected & 0o111 != 0) != file.executable)
    {
        return Err(ApplyError::ContentMismatch);
    }
    if entry.kind() == DeltaKind::Added {
        workspace
            .create_new(entry.path(), &file.bytes)
            .map_err(|_| ApplyError::Fs)?;
    } else {
        let baseline_hex = baseline_hex(baseline, entry.path())?;
        workspace
            .write_guarded(entry.path(), baseline_hex, &file.bytes)
            .map_err(|_| ApplyError::Fs)?;
    }
    super::apply_fs::set_candidate_mode_bits(
        workspace,
        entry.path(),
        mode.unwrap_or(if file.executable { 0o755 } else { 0o644 }),
    )?;
    Ok(())
}

/// The baseline digest for a path, failing closed when the baseline has none.
fn baseline_hex(baseline: &CapturedSnapshot, path: &str) -> Result<&str, ApplyError> {
    baseline
        .entries()
        .iter()
        .find(|entry| entry.path().as_str() == path)
        .map(|entry| entry.content_sha256().as_str())
        .ok_or(ApplyError::ContentMismatch)
}
