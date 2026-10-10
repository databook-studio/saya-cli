//! Integration receipts: shared assigned-path contracts and the actual checkout delta.

mod delta;
mod delta_types;
mod delta_walk;
mod paths;
mod receipt;

pub use delta::compute_delta;
pub use delta_types::{CheckoutDelta, DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, PathDelta};
pub use paths::{
    AssignedPaths, AssignedPathsError, MAX_ASSIGNED_PATH_BYTES, MAX_ASSIGNED_PATHS,
    MAX_ASSIGNED_PATHS_TOTAL_BYTES, casefold_collision,
};
pub use receipt::{
    ChangeReceipt, ChangeScope, ReceiptChange, ReceiptOutcome, RejectionReason, evaluate_receipt,
};
