//! Integration receipts: shared assigned-path contracts and the actual checkout delta.

mod apply;
mod apply_fs;
mod candidate;
mod conflicts;
mod conflicts_types;
mod delta;
mod delta_output;
mod delta_types;
mod delta_walk;
mod evidence;
mod index;
mod integrate;
mod integrate_events;
mod paths;
mod preflight;
mod preflight_fs;
mod receipt;
mod transaction;
mod transaction_events;
mod transaction_identity;
mod transaction_images;
mod transaction_journal;
mod transaction_journal_io;

pub use apply::{
    AcceptedDelta, ApplyError, OverlapConflict, OverlapPair, apply_delta, detect_overlaps,
};
pub use candidate::{CandidateError, CandidateIdentity, candidate_source_digest};
pub use conflicts::{
    ConflictDecision, ConflictError, ConflictQueue, ConflictResolution, MAX_RETAINED_CONFLICTS,
};
pub use conflicts_types::{RetainedConflict, RetainedDeltaInput, RetainedPair};
pub use delta::compute_delta;
pub use delta_types::{CheckoutDelta, DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, PathDelta};
pub use evidence::{EvidenceBinding, EvidenceError};
pub use index::{PlanError, PlanIndex, PlanIndexEntry, PlanStatusFact};
pub use integrate::{IntegrateError, IntegrateInput, IntegratedCandidate, integrate_candidate};
pub use paths::{
    AssignedPaths, AssignedPathsError, MAX_ASSIGNED_PATH_BYTES, MAX_ASSIGNED_PATHS,
    MAX_ASSIGNED_PATHS_TOTAL_BYTES, casefold_collision,
};
pub use preflight::{
    ApplyBackPreflight, Collision, CollisionKind, PreflightBounds, PreflightError, PreflightInput,
    preflight_apply_back,
};
pub use receipt::{
    ChangeReceipt, ChangeScope, ReceiptChange, ReceiptOutcome, RejectionReason, evaluate_receipt,
};
pub use transaction::{
    AppliedApplyBack, ApplyBackBounds, ApplyBackError, ApplyBackInput, apply_back,
};
pub use transaction_journal::{JournalError, apply_back_latest_dir};
