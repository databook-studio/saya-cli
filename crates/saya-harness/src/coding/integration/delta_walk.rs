//! The live-checkout walk behind every delta: it fingerprints the files the
//! workspace currently holds. The Unix implementation is anchored — every
//! open is relative to a descriptor for the entry's parent with `O_NOFOLLOW`,
//! so a swapped symlink is refused instead of followed.
//!
//! The walk follows the checkout's own Git ignore rules and never fails on a
//! single path it cannot represent: an ignored directory is skipped whole, a
//! special or nested-repository path is reported as not included, and a bound
//! trip stops the walk with a reported partial result. Cancellation and I/O
//! failures still refuse the whole walk.

#[cfg(not(windows))]
use std::collections::HashSet;
#[cfg(not(windows))]
use std::path::Path;
#[cfg(not(windows))]
use std::time::Instant;

#[cfg(not(windows))]
use saya_agent::CancellationToken;

#[cfg(not(windows))]
use super::delta_types::DeltaError;
#[cfg(not(windows))]
use super::delta_types::WalkOutcome;
#[cfg(not(windows))]
use crate::coding::git::IgnoredPaths;

#[cfg(unix)]
#[path = "delta_walk_retry.rs"]
mod retry;

#[cfg(unix)]
#[path = "delta_walk_state.rs"]
mod state;

#[cfg(unix)]
#[path = "delta_walk_unix.rs"]
mod unix;

#[cfg(windows)]
#[path = "delta_walk_windows.rs"]
mod windows;

#[cfg(windows)]
pub(super) use windows::walk_checkout_with_root_git_directory;

/// Walks the checkout rooted at `root` and returns every live file it could
/// include, sorted by path. Every access is relative to a descriptor for its
/// parent and no-follow, so a swapped symlink is refused instead of followed;
/// sizes and the executable bit come from the opened file; bounds count bytes
/// actually read. Descent is depth-first, so only the ancestor directory
/// streams are open. The root-level `.saya` directory is an authorized output
/// root that is bounded and counted but never classified or digested.
///
/// The checkout is live, so an entry that leaves it between the listing and the
/// read is not a refusal: the walk is tried again, bounded, and settles on the
/// tree that remains.
#[cfg(unix)]
pub(super) fn walk_checkout_with_root_git_directory(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
    allow_root_git_directory: bool,
    omitted: &HashSet<String>,
    ignored: &IgnoredPaths,
) -> Result<WalkOutcome, DeltaError> {
    retry::walk_until_settled(|| {
        unix::walk_once(
            root,
            cancellation,
            deadline,
            allow_root_git_directory,
            omitted,
            ignored,
        )
    })
}

/// A directory nesting deeper than this bound is refused as an unsafe path,
/// and it keeps the ancestor directory streams well under the process
/// descriptor limit.
#[cfg(any(unix, windows))]
const MAX_WALK_DEPTH: usize = 64;

/// Hosts with neither the anchored walk nor the Windows path walk refuse.
#[cfg(all(not(unix), not(windows)))]
pub(super) fn walk_checkout_with_root_git_directory(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
    allow_root_git_directory: bool,
    omitted: &HashSet<String>,
    ignored: &IgnoredPaths,
) -> Result<WalkOutcome, DeltaError> {
    let _ = (
        root,
        cancellation,
        deadline,
        allow_root_git_directory,
        omitted,
        ignored,
    );
    Err(DeltaError::Io)
}
