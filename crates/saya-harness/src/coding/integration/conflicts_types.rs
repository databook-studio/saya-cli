//! The retained value types of the conflict queue: identities, digests and counts only, never bytes or paths in Debug

use saya_types::{ManagedCheckoutId, Sha256Digest};
use std::fmt;

/// One retained input: child identity plus the receipt's content digests.
///
/// `AcceptedDelta::new` already proved the receipt matches the delta one equal
/// change per entry, so the delta entries' `content_sha256` values ARE the
/// receipt's digests. Holds ids, digests and counts only — never bytes.
#[derive(Clone)]
pub struct RetainedDeltaInput {
    pub(super) checkout_id: ManagedCheckoutId,
    pub(super) content_sha256s: Vec<Option<Sha256Digest>>,
    pub(super) total_bytes: u64,
}

impl RetainedDeltaInput {
    /// The child checkout id this accepted delta was verified against.
    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    /// Per-entry content digests, in entry order.
    pub fn content_sha256s(&self) -> &[Option<Sha256Digest>] {
        &self.content_sha256s
    }

    /// The delta's total byte count at acceptance time.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

impl fmt::Debug for RetainedDeltaInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedDeltaInput")
            .field("checkout_id", &self.checkout_id)
            .field("entries", &self.content_sha256s.len())
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

/// Copy of one overlap pair; `OverlapPair`'s fields are private and only
/// `detect_overlaps` constructs it, so the queue retains its own value with
/// the same shape. [`std::fmt::Debug`] prints the indexes only.
#[derive(Clone)]
pub struct RetainedPair {
    pub(super) first: usize,
    pub(super) second: usize,
    pub(super) path: String,
}

impl RetainedPair {
    /// Index into the accepted slice of the pair's first member.
    pub fn first(&self) -> usize {
        self.first
    }

    /// Index into the accepted slice of the pair's second member.
    pub fn second(&self) -> usize {
        self.second
    }

    /// The conflicting path. Never printed via `Debug`.
    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Debug for RetainedPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedPair")
            .field("first", &self.first)
            .field("second", &self.second)
            .finish()
    }
}

/// One retained conflict: the candidate's identity, the overlapping pairs, and
/// both inputs mapped from the accepted slice. [`std::fmt::Debug`] prints the
/// candidate id and counts only — never paths or digests.
#[derive(Clone)]
pub struct RetainedConflict {
    pub(super) candidate_checkout_id: ManagedCheckoutId,
    pub(super) pairs: Vec<RetainedPair>,
    pub(super) inputs: Vec<RetainedDeltaInput>,
}

impl RetainedConflict {
    /// The candidate whose submission raised the conflict.
    pub fn candidate_checkout_id(&self) -> &ManagedCheckoutId {
        &self.candidate_checkout_id
    }

    /// The overlapping pairs, deduplicated as in `detect_overlaps`.
    pub fn pairs(&self) -> &[RetainedPair] {
        &self.pairs
    }

    /// Both retained inputs, in accepted-slice order.
    pub fn inputs(&self) -> &[RetainedDeltaInput] {
        &self.inputs
    }
}

impl fmt::Debug for RetainedConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetainedConflict")
            .field("candidate_checkout_id", &self.candidate_checkout_id)
            .field("pairs", &self.pairs.len())
            .field("inputs", &self.inputs.len())
            .finish()
    }
}
