//! How a delta answers a checkout that moves while it is being read.
//!
//! A working checkout is live: the root, a helper, a hook or the sandbox
//! probe's own cleanup can create and remove paths at any moment. A path the
//! walk has already listed and that is gone when the walk reads it is a normal
//! moment of that life, not a checkout that cannot be read — so the delta walks
//! the tree again and settles on what remains.
//!
//! The retry is bounded and immediate: the tree either holds still for one more
//! walk or it does not, and three walks that in a row lose an entry refuse the
//! delta instead of chasing a tree that will not be fingerprinted.

use super::super::delta_types::{DeltaError, WalkOutcome};

/// Most walks one delta makes when entries keep vanishing under it.
pub(super) const MAX_WALK_ATTEMPTS: u8 = 3;

/// Walks until an attempt is not refused by an entry that vanished between the
/// listing and the read, bounded to [`MAX_WALK_ATTEMPTS`] walks in total. Every
/// other refusal — cancellation, a deadline, a bound, an unreadable checkout —
/// is returned from the walk that produced it.
pub(super) fn walk_until_settled<F>(mut walk: F) -> Result<WalkOutcome, DeltaError>
where
    F: FnMut() -> Result<WalkOutcome, DeltaError>,
{
    let mut attempt: u8 = 1;
    loop {
        let outcome = walk();
        if attempt >= MAX_WALK_ATTEMPTS || !matches!(outcome, Err(DeltaError::Vanished)) {
            return outcome;
        }
        attempt += 1;
    }
}
