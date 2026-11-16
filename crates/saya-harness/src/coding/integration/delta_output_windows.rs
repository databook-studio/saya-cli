//! The Windows scan of the checkout's authorized output root. The subtree is
//! counted and bounded exactly as on unix, but the walk is path-based and
//! refuses reparse points instead of opening anchored descriptors.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use saya_agent::CancellationToken;

use super::super::delta_types::DeltaError;
use super::{MAX_OUTPUT_ROOT_BYTES, MAX_OUTPUT_ROOT_DEPTH, MAX_OUTPUT_ROOT_ENTRIES};
use crate::workspace::windows_identity;

/// Scans the output-root subtree at `root` and refuses anything over its
/// bound or hostile. Entries are counted, regular-file lengths are totalled
/// from `symlink_metadata`, descent keeps to the depth bound, and a reparse
/// point (a link or junction) is refused rather than followed.
pub(crate) fn scan_output_root(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), DeltaError> {
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut entries: usize = 0;
    let mut total_bytes: u64 = 0;
    while let Some((directory, depth)) = stack.pop() {
        check_gate(cancellation, deadline)?;
        let read = fs::read_dir(&directory).map_err(|_| DeltaError::Io)?;
        for entry in read {
            let entry = entry.map_err(|_| DeltaError::Io)?;
            check_gate(cancellation, deadline)?;
            entries += 1;
            if entries > MAX_OUTPUT_ROOT_ENTRIES {
                return Err(DeltaError::OutputRootTooLarge);
            }
            let name = entry.file_name();
            if name.to_string_lossy().eq_ignore_ascii_case(".git") {
                return Err(DeltaError::NestedRepository);
            }
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|_| DeltaError::Io)?;
            if windows_identity::path_is_reparse_point(&path).map_err(|_| DeltaError::Io)? {
                return Err(DeltaError::Symlink);
            }
            if meta.is_dir() {
                if depth + 1 >= MAX_OUTPUT_ROOT_DEPTH {
                    return Err(DeltaError::OutputRootTooLarge);
                }
                stack.push((path, depth + 1));
            } else if meta.is_file() {
                total_bytes = total_bytes
                    .checked_add(meta.len())
                    .ok_or(DeltaError::OutputRootTooLarge)?;
                if total_bytes > MAX_OUTPUT_ROOT_BYTES {
                    return Err(DeltaError::OutputRootTooLarge);
                }
            } else {
                return Err(DeltaError::Symlink);
            }
        }
    }
    Ok(())
}

fn check_gate(cancellation: &CancellationToken, deadline: Instant) -> Result<(), DeltaError> {
    if cancellation.is_cancelled() {
        return Err(DeltaError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(DeltaError::DeadlineExceeded);
    }
    Ok(())
}
