//! The checkout's authorized output root: the root-level `.saya` directory is
//! bounded, evidence-free space — its contents are neither classified nor
//! digested, only counted — so confined child commands can write their
//! private `HOME`/`TMPDIR` trees there without turning the run into an
//! `OutsideAssignment` receipt.

use std::ffi::OsStr;
use std::time::Instant;

use saya_agent::CancellationToken;

use super::delta_types::DeltaError;

#[cfg(unix)]
use crate::workspace::fd::{DirStream, stat_at};
#[cfg(unix)]
use std::os::fd::RawFd;

/// The root-level directory reserved as the checkout's output root: bounded
/// evidence-free space whose contents are neither classified nor digested,
/// only counted.
pub(super) const OUTPUT_ROOT_NAME: &str = ".saya";

/// The most entries (files and directories) the output root may hold.
pub(super) const MAX_OUTPUT_ROOT_ENTRIES: usize = 4096;

/// The most bytes of regular-file content the output root may hold.
pub(super) const MAX_OUTPUT_ROOT_BYTES: u64 = 64 * 1024 * 1024;

/// A directory nesting deeper than this bound exceeds the output root's
/// bound, and it keeps the ancestor directory streams well under the process
/// descriptor limit.
#[cfg(unix)]
const MAX_OUTPUT_ROOT_DEPTH: usize = 16;

/// Scans the output-root subtree anchored at `name` under `parent_fd` and
/// refuses anything over its bound or hostile. The scan is iterative and
/// depth-first, so only the ancestor directory streams are open, and it never
/// opens or hashes a file: entries are counted against
/// [`MAX_OUTPUT_ROOT_ENTRIES`], regular-file lengths are totalled against
/// [`MAX_OUTPUT_ROOT_BYTES`] from the anchored `lstat`, and descent keeps to
/// [`MAX_OUTPUT_ROOT_DEPTH`]. Non-UTF-8 names are fine here — the subtree is
/// never reported — so names are used as [`OsStr`]s throughout.
#[cfg(unix)]
pub(super) fn scan_output_root(
    parent_fd: RawFd,
    name: &OsStr,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), DeltaError> {
    let root_stream =
        DirStream::open_at(parent_fd, name).map_err(|error| match error.raw_os_error() {
            Some(libc::ELOOP) | Some(libc::ENOTDIR) => DeltaError::Symlink,
            _ => DeltaError::Io,
        })?;
    let mut stack: Vec<DirStream> = vec![root_stream];
    let mut entries: usize = 0;
    let mut total_bytes: u64 = 0;

    while let Some(stream) = stack.last_mut() {
        check_gate(cancellation, deadline)?;
        let Some((entry, _d_type)) = stream.next().map_err(|_| DeltaError::Io)? else {
            stack.pop();
            continue;
        };
        entries += 1;
        if entries > MAX_OUTPUT_ROOT_ENTRIES {
            return Err(DeltaError::OutputRootTooLarge);
        }
        if entry.eq_ignore_ascii_case(".git") {
            return Err(DeltaError::NestedRepository);
        }
        let dir_fd = stream.dir_fd();
        let st = stat_at(dir_fd, entry.as_os_str()).map_err(|_| DeltaError::Io)?;
        if st.is_dir() {
            if stack.len() >= MAX_OUTPUT_ROOT_DEPTH {
                return Err(DeltaError::OutputRootTooLarge);
            }
            let child =
                DirStream::open_at(dir_fd, entry.as_os_str()).map_err(|error| {
                    match error.raw_os_error() {
                        Some(libc::ELOOP) | Some(libc::ENOTDIR) => DeltaError::Symlink,
                        _ => DeltaError::Io,
                    }
                })?;
            stack.push(child);
        } else if st.is_file() {
            total_bytes = total_bytes
                .checked_add(st.len())
                .ok_or(DeltaError::OutputRootTooLarge)?;
            if total_bytes > MAX_OUTPUT_ROOT_BYTES {
                return Err(DeltaError::OutputRootTooLarge);
            }
        } else {
            return Err(DeltaError::Symlink);
        }
    }
    Ok(())
}

/// The scan's one gate: cancelled beats expired, and both beat success.
#[cfg(unix)]
fn check_gate(cancellation: &CancellationToken, deadline: Instant) -> Result<(), DeltaError> {
    if cancellation.is_cancelled() {
        return Err(DeltaError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(DeltaError::DeadlineExceeded);
    }
    Ok(())
}

/// Non-Unix hosts have no anchored scan — no descriptor-relative, no-follow
/// opens to lean on — and the coding feature is Unix-only today, so the scan
/// refuses.
#[cfg(not(unix))]
pub(super) fn scan_output_root(
    parent_fd: i32,
    name: &OsStr,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), DeltaError> {
    let _ = (parent_fd, name, cancellation, deadline);
    Err(DeltaError::Io)
}
