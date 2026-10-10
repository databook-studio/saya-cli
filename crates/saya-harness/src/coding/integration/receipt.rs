//! The change receipt: a pure verdict over the actual checkout delta, scoped by the
//! assigned-path set.
//!
//! The verdict is computed only from the [`VerifiedCheckout`] identity, the
//! [`CheckoutDelta`] (W07b) and the [`AssignedPaths`] set (W07a). It is independent of
//! any child claim — there is no claim parameter at all — and it is not a correctness
//! verdict: it judges scope (owned vs. unowned paths) and size against the byte bound
//! only, never whether the changes are right.

use saya_types::{ManagedCheckoutId, Sha256Digest, SnapshotId};

use crate::coding::worktree::VerifiedCheckout;

use super::{AssignedPaths, CheckoutDelta, DeltaKind};

/// Whether a change touches a path inside the assigned-path set or outside it.
///
/// A rename is a [`DeltaKind::Deleted`] plus an [`DeltaKind::Added`]; each endpoint is
/// judged against the assignment separately, so a rename to or from an unowned path
/// always yields an unowned change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeScope {
    /// The path is covered by the assigned-path set.
    Owned,
    /// The path is not covered by the assigned-path set.
    Unowned,
}

/// The verdict of a receipt: scope and size only, never correctness.
///
/// [`ReceiptOutcome::WithinAssignment`] does not mean "accepted" and does not assert
/// that the changes are correct; it means every change was in scope and the byte bound
/// was respected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptOutcome {
    /// Every change was owned and the total byte count respected `max_bytes`.
    WithinAssignment,
    /// At least one change was unowned or the byte bound was exceeded.
    OutsideAssignment,
}

/// Why the receipt ruled [`ReceiptOutcome::OutsideAssignment`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// At least one change touched an unowned path.
    UnownedPath,
    /// The total byte count exceeded the configured bound.
    OverByteBound,
}

/// One change in a receipt, derived from a delta entry plus its assignment scope.
///
/// The [`std::fmt::Debug`] representation never prints the path.
#[derive(Clone, PartialEq, Eq)]
pub struct ReceiptChange {
    path: String,
    kind: DeltaKind,
    bytes: u64,
    content_sha256: Option<Sha256Digest>,
    scope: ChangeScope,
}

impl ReceiptChange {
    /// The path the change applies to.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The kind of change.
    pub fn kind(&self) -> DeltaKind {
        self.kind
    }

    /// The size in bytes of the file after the change (zero for deletions).
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The content digest after the change, if the file exists.
    pub fn content_sha256(&self) -> Option<&Sha256Digest> {
        self.content_sha256.as_ref()
    }

    /// Whether the change is owned by the assigned-path set.
    pub fn scope(&self) -> ChangeScope {
        self.scope
    }
}

impl std::fmt::Debug for ReceiptChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiptChange")
            .field("kind", &self.kind)
            .field("bytes", &self.bytes)
            .field("scope", &self.scope)
            .finish()
    }
}

/// The full receipt for a verified checkout: identity, verdict, reasons, and changes.
///
/// The [`std::fmt::Debug`] representation prints the checkout identity, outcome, and
/// counts, and never prints a path.
#[derive(Clone, PartialEq, Eq)]
pub struct ChangeReceipt {
    checkout_id: ManagedCheckoutId,
    snapshot_id: SnapshotId,
    snapshot_sha256: Sha256Digest,
    plan_sha256: Sha256Digest,
    outcome: ReceiptOutcome,
    reasons: Vec<RejectionReason>,
    changes: Vec<ReceiptChange>,
    unowned_count: usize,
    total_bytes: u64,
}

impl ChangeReceipt {
    /// The managed checkout this receipt describes.
    pub fn checkout_id(&self) -> &ManagedCheckoutId {
        &self.checkout_id
    }

    /// The snapshot the checkout was verified against.
    pub fn snapshot_id(&self) -> &SnapshotId {
        &self.snapshot_id
    }

    /// The root digest of the verified snapshot.
    pub fn snapshot_sha256(&self) -> &Sha256Digest {
        &self.snapshot_sha256
    }

    /// The plan digest the checkout was verified against.
    pub fn plan_sha256(&self) -> &Sha256Digest {
        &self.plan_sha256
    }

    /// The verdict on scope and size.
    pub fn outcome(&self) -> ReceiptOutcome {
        self.outcome
    }

    /// The reasons for [`ReceiptOutcome::OutsideAssignment`], empty when within.
    pub fn reasons(&self) -> &[RejectionReason] {
        &self.reasons
    }

    /// The changes in delta order (sorted by path).
    pub fn changes(&self) -> &[ReceiptChange] {
        &self.changes
    }

    /// How many changes touched unowned paths.
    pub fn unowned_count(&self) -> usize {
        self.unowned_count
    }

    /// The total byte count of the delta.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

impl std::fmt::Debug for ChangeReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeReceipt")
            .field("checkout_id", &self.checkout_id)
            .field("outcome", &self.outcome)
            .field("reason_count", &self.reasons.len())
            .field("change_count", &self.changes.len())
            .field("unowned_count", &self.unowned_count)
            .field("total_bytes", &self.total_bytes)
            .finish()
    }
}

/// Evaluate the receipt over the actual delta against the assigned-path set.
///
/// For each delta entry in order, the scope is [`ChangeScope::Owned`] iff the assigned
/// set contains the entry's path, else [`ChangeScope::Unowned`]. The reasons are
/// [`RejectionReason::UnownedPath`] when any entry is unowned (reported first), and
/// [`RejectionReason::OverByteBound`] when the delta's total bytes exceed `max_bytes`.
/// The outcome is [`ReceiptOutcome::WithinAssignment`] iff no reasons were recorded, so
/// an empty delta is always within assignment.
pub fn evaluate_receipt(
    verified: &VerifiedCheckout,
    delta: &CheckoutDelta,
    assigned: &AssignedPaths,
    max_bytes: u64,
) -> ChangeReceipt {
    let mut changes = Vec::with_capacity(delta.len());
    let mut unowned_count = 0usize;
    for entry in delta.entries() {
        let scope = if assigned.contains(entry.path()) {
            ChangeScope::Owned
        } else {
            unowned_count += 1;
            ChangeScope::Unowned
        };
        changes.push(ReceiptChange {
            path: entry.path().to_owned(),
            kind: entry.kind(),
            bytes: entry.bytes(),
            content_sha256: entry.content_sha256().cloned(),
            scope,
        });
    }

    let total_bytes = delta.total_bytes();
    let mut reasons = Vec::new();
    if unowned_count > 0 {
        reasons.push(RejectionReason::UnownedPath);
    }
    if total_bytes > max_bytes {
        reasons.push(RejectionReason::OverByteBound);
    }
    let outcome = if reasons.is_empty() {
        ReceiptOutcome::WithinAssignment
    } else {
        ReceiptOutcome::OutsideAssignment
    };

    ChangeReceipt {
        checkout_id: verified.checkout_id().clone(),
        snapshot_id: verified.snapshot_id().clone(),
        snapshot_sha256: verified.snapshot_sha256().clone(),
        plan_sha256: verified.plan_sha256().clone(),
        outcome,
        reasons,
        changes,
        unowned_count,
        total_bytes,
    }
}
