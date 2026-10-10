//! Conflict retention and explicit resolution for overlapping accepted deltas.
//!
//! When `integrate_candidate` rejects a candidate with an [`OverlapConflict`],
//! [`ConflictQueue::record`] retains BOTH conflicting inputs — by identity and
//! content digests only, never file bytes. Resolution is never automatic: a
//! caller-supplied [`ConflictDecision`] yields which accepted indexes survive
//! ([`ConflictResolution::KeptIndexes`]) and the CALLER re-submits them through
//! `integrate_candidate`. `ManualEdit` routes to a full re-derive and a fresh
//! submission of the edited child.
//!
//! Retained records are process-local and lost on restart. A record keyed to a
//! terminal (crash-open) candidate id can still be resolved in memory, but the
//! resulting plan must be applied through a rebuilt candidate with a fresh
//! checkout id; never retry the terminal id.

use super::conflicts_types::{RetainedConflict, RetainedDeltaInput, RetainedPair};
use super::{AcceptedDelta, OverlapConflict};
use saya_types::ManagedCheckoutId;

/// Hard cap on retained conflict records; the queue fails closed past it.
pub const MAX_RETAINED_CONFLICTS: usize = 64;

/// Errors raised while retaining or resolving a conflict.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error("the conflict queue is full")]
    QueueFull,
    #[error("no open conflict is retained for this candidate")]
    NotOpen,
    #[error("the conflict is already resolved")]
    AlreadyResolved,
    #[error("the decision contradicts the retained pairs")]
    AmbiguousDecision,
}

/// Typed explicit decisions; no automatic resolution.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictDecision {
    /// Keep every pair's `first` member; drop every `second`.
    KeepFirst,
    /// Keep every pair's `second` member; drop every `first`.
    KeepSecond,
    /// The candidate tree was edited: a fresh delta must be re-derived
    /// (`compute_delta` → `evaluate_receipt` → `AcceptedDelta::new`) and
    /// re-submitted as a fresh submission.
    ManualEdit,
}

/// What a decision produces: surviving accepted indexes (the caller
/// re-submits them through `integrate_candidate`), or a full manual re-edit.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictResolution {
    /// Accepted indexes that survive the decision, ascending.
    KeptIndexes(Vec<usize>),
    /// The caller re-derives a delta from the edited child and resubmits.
    ManualEdit,
}

/// One queue entry: the retained conflict plus its resolution state.
struct Record {
    conflict: RetainedConflict,
    resolved: bool,
}

/// In-memory, bounded, per-process conflict retention. Pure: no store, no fs,
/// no async. The queue never mutates anything on its own — the only mutation
/// is a caller-supplied [`ConflictDecision`].
pub struct ConflictQueue {
    records: Vec<Record>,
}

impl Default for ConflictQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl ConflictQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    /// Maps the conflict's indexes to identities + delta/receipt content digests.
    ///
    /// One [`RetainedDeltaInput`] per accepted delta; one [`RetainedPair`] per
    /// overlap pair. A record for the same candidate id is REPLACED (latest
    /// wins, unresolved); otherwise a queue at [`MAX_RETAINED_CONFLICTS`] fails
    /// closed with [`ConflictError::QueueFull`].
    pub fn record(
        &mut self,
        candidate: &ManagedCheckoutId,
        accepted: &[AcceptedDelta<'_>],
        conflict: &OverlapConflict,
    ) -> Result<(), ConflictError> {
        let pairs = conflict
            .pairs()
            .iter()
            .map(|pair| RetainedPair {
                first: pair.first(),
                second: pair.second(),
                path: pair.path().to_owned(),
            })
            .collect();
        let inputs = accepted
            .iter()
            .map(|delta| RetainedDeltaInput {
                checkout_id: delta.verified().checkout_id().clone(),
                content_sha256s: delta
                    .delta()
                    .entries()
                    .iter()
                    .map(|entry| entry.content_sha256().cloned())
                    .collect(),
                total_bytes: delta.delta().total_bytes(),
            })
            .collect();
        let record = Record {
            conflict: RetainedConflict {
                candidate_checkout_id: candidate.clone(),
                pairs,
                inputs,
            },
            resolved: false,
        };
        match self
            .records
            .iter_mut()
            .find(|existing| &existing.conflict.candidate_checkout_id == candidate)
        {
            Some(existing) => *existing = record,
            None => {
                if self.records.len() >= MAX_RETAINED_CONFLICTS {
                    return Err(ConflictError::QueueFull);
                }
                self.records.push(record);
            }
        }
        Ok(())
    }

    /// The retained conflict for `candidate`, only while it is unresolved.
    pub fn open_for(&self, candidate: &ManagedCheckoutId) -> Option<&RetainedConflict> {
        self.records
            .iter()
            .find(|record| !record.resolved && &record.conflict.candidate_checkout_id == candidate)
            .map(|record| &record.conflict)
    }

    /// KeepFirst/KeepSecond drop the `second`/`first` index of every pair; an
    /// index both kept and dropped (chain A–B, B–C) is AmbiguousDecision.
    /// ManualEdit needs no indexes. On success the record is marked resolved;
    /// the surviving indexes are the caller's plan for resubmission — the
    /// queue never rebuilds or mutates an `AcceptedDelta` itself.
    pub fn resolve(
        &mut self,
        candidate: &ManagedCheckoutId,
        decision: ConflictDecision,
    ) -> Result<ConflictResolution, ConflictError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| &record.conflict.candidate_checkout_id == candidate)
            .ok_or(ConflictError::NotOpen)?;
        if record.resolved {
            return Err(ConflictError::AlreadyResolved);
        }
        let resolution = match decision {
            ConflictDecision::ManualEdit => ConflictResolution::ManualEdit,
            ConflictDecision::KeepFirst => {
                ConflictResolution::KeptIndexes(surviving_indexes(&record.conflict, true)?)
            }
            ConflictDecision::KeepSecond => {
                ConflictResolution::KeptIndexes(surviving_indexes(&record.conflict, false)?)
            }
        };
        record.resolved = true;
        Ok(resolution)
    }
}

/// Indexes of the retained inputs that survive a keep-first/keep-second
/// decision, ascending. KeepFirst drops every pair's `second` index;
/// KeepSecond drops every pair's `first` index. An index both kept and dropped
/// (chain A–B, B–C) contradicts the decision and fails closed with
/// [`ConflictError::AmbiguousDecision`].
fn surviving_indexes(
    conflict: &RetainedConflict,
    keep_first: bool,
) -> Result<Vec<usize>, ConflictError> {
    let dropped: Vec<usize> = conflict
        .pairs()
        .iter()
        .map(|pair| if keep_first { pair.second } else { pair.first })
        .collect();
    let contradicts = conflict
        .pairs()
        .iter()
        .any(|pair| dropped.contains(&if keep_first { pair.first } else { pair.second }));
    if contradicts {
        return Err(ConflictError::AmbiguousDecision);
    }
    Ok((0..conflict.inputs().len())
        .filter(|index| !dropped.contains(index))
        .collect())
}
