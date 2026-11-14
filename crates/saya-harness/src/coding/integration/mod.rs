//! Integration receipts: shared assigned-path contracts and the actual checkout delta.

mod apply;
mod apply_fs;
mod candidate;
mod delta;
#[cfg(unix)]
mod delta_output;
mod delta_types;
mod delta_walk;
mod evidence;
mod index;
mod integrate;
mod integrate_events;
mod merge;
mod paths;
mod preflight;
#[cfg(unix)]
mod preflight_fs;
mod receipt;
mod recover;
mod recover_binding;
mod recover_files;
mod recover_journal;
mod transaction;
mod transaction_events;
mod transaction_identity;
mod transaction_images;
mod transaction_journal;
mod transaction_journal_io;
mod transaction_lease;
#[cfg(all(test, unix))]
mod transaction_tests;

pub use apply::{
    AcceptedDelta, ApplyError, OverlapConflict, OverlapPair, apply_delta, detect_overlaps,
};
pub use candidate::{
    CandidateError, CandidateIdentity, candidate_source_digest, managed_candidate_source_digest,
    operator_project_source_digest,
};
pub use delta::{compute_delta, compute_delta_at};
pub use delta_types::{
    CheckoutDelta, DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, NotIncludedPath,
    NotIncludedReason, PathDelta,
};
pub use evidence::{EvidenceBinding, EvidenceError};
pub use index::{PlanError, PlanIndex, PlanIndexEntry, PlanStatusFact};
pub use integrate::{IntegrateError, IntegrateInput, IntegratedCandidate, integrate_candidate};
pub use merge::{MergeError, MergeInput, MergedIntoWorking, merge_into_working};
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
pub use recover::{RecoverError, RecoveryMode, RecoveryOutcome, recover_apply_back};
pub use transaction::{
    AppliedApplyBack, ApplyBackBounds, ApplyBackError, ApplyBackInput, apply_back,
};
pub use transaction_journal::{JournalError, apply_back_latest_dir};
