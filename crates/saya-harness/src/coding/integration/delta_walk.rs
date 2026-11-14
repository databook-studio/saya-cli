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

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use saya_agent::CancellationToken;

use super::delta_types::{DeltaError, WalkOutcome};
#[cfg(unix)]
use super::delta_types::{LiveFile, NotIncludedReason};
use crate::coding::git::IgnoredPaths;

#[cfg(unix)]
#[path = "delta_walk_state.rs"]
mod state;
#[cfg(unix)]
use crate::coding::snapshot::{SnapshotFileMode, SnapshotPath};
#[cfg(unix)]
use crate::workspace::fd::{DirStream, open_root_fd, stat_at};
#[cfg(unix)]
use state::{WalkState, check_gate, hash_file, hash_symlink, record};
#[cfg(unix)]
use std::{ffi::OsStr, os::fd::AsRawFd};

/// Walks the checkout rooted at `root` and returns every live file it could
/// include, sorted by path. Every access is relative to a descriptor for its
/// parent and no-follow, so a swapped symlink is refused instead of followed;
/// sizes and the executable bit come from the opened file; bounds count bytes
/// actually read. Descent is depth-first, so only the ancestor directory
/// streams are open. The root-level `.saya` directory is an authorized output
/// root that is bounded and counted but never classified or digested.
#[cfg(unix)]
pub(super) fn walk_checkout_with_root_git_directory(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
    allow_root_git_directory: bool,
    omitted: &HashSet<String>,
    ignored: &IgnoredPaths,
) -> Result<WalkOutcome, DeltaError> {
    let root_fd = open_root_fd(root.as_os_str()).map_err(|_| DeltaError::Io)?;
    let root_stream =
        DirStream::open_at(root_fd.as_raw_fd(), OsStr::new(".")).map_err(|_| DeltaError::Io)?;
    let mut stack: Vec<(DirStream, String)> = vec![(root_stream, String::new())];
    let mut files: Vec<LiveFile> = Vec::new();
    let mut state = WalkState::default();
    let mut aborted = None;

    while let Some((stream, prefix)) = stack.last_mut() {
        if let Err(error) = check_gate(cancellation, deadline) {
            if error == DeltaError::Cancelled {
                return Err(error);
            }
            aborted = Some(error);
            break;
        }
        let Some((name, _d_type)) = stream.next().map_err(|_| DeltaError::Io)? else {
            stack.pop();
            continue;
        };
        let name = match name.to_str() {
            Some(name) => name.to_owned(),
            None => return Err(DeltaError::UnsafePath),
        };
        let is_root = prefix.is_empty();
        let dir_fd = stream.dir_fd();
        let relative = if is_root {
            name.clone()
        } else {
            format!("{}/{}", prefix.as_str(), name)
        };
        // The root `.saya` output root is scanned and bounded even though the
        // baseline omits it, so scratch growth cannot go unnoticed.
        if is_root && name == super::delta_output::OUTPUT_ROOT_NAME {
            let st = stat_at(dir_fd, OsStr::new(&name)).map_err(|_| DeltaError::Io)?;
            if st.is_dir() {
                super::delta_output::scan_output_root(
                    dir_fd,
                    OsStr::new(&name),
                    cancellation,
                    deadline,
                )?;
                continue;
            }
        }
        // The baseline's own omissions are not job content. Skipping them
        // here keeps a sensitive or nested path the planner omitted from
        // appearing as created, and skips a whole omitted directory.
        if omitted.contains(relative.as_str()) {
            continue;
        }
        // The checkout's own ignore rules decide what is job content. An
        // ignored directory marker skips its whole subtree, so generated
        // output never enters the delta or trips a file bound.
        if ignored.covers(Path::new(&relative)) {
            continue;
        }
        if name.eq_ignore_ascii_case(".git") {
            let metadata = if is_root && name == ".git" {
                Some(stat_at(dir_fd, OsStr::new(&name)).map_err(|_| DeltaError::Io)?)
            } else {
                None
            };
            if metadata
                .is_some_and(|entry| entry.is_file() || allow_root_git_directory && entry.is_dir())
            {
                continue;
            }
            // A nested repository marker is metadata, not job content: report
            // it and keep walking its siblings.
            state.not_included(&relative, NotIncludedReason::NestedRepository);
            continue;
        }
        if SnapshotPath::parse(relative.clone()).is_err() {
            state.not_included(&relative, NotIncludedReason::UnsafePath);
            continue;
        }
        let st = stat_at(dir_fd, OsStr::new(&name)).map_err(|_| DeltaError::Io)?;
        if st.is_dir() {
            if stack.len() >= MAX_WALK_DEPTH {
                state.not_included(&relative, NotIncludedReason::UnsafePath);
                continue;
            }
            let child = match DirStream::open_at(dir_fd, OsStr::new(&name)) {
                Ok(child) => child,
                Err(error) => match error.raw_os_error() {
                    Some(libc::ELOOP) | Some(libc::ENOTDIR) => {
                        state.not_included(&relative, NotIncludedReason::SpecialFile);
                        continue;
                    }
                    _ => return Err(DeltaError::Io),
                },
            };
            stack.push((child, relative));
        } else if st.is_file() {
            match hash_file(dir_fd, OsStr::new(&name), cancellation, deadline, &state) {
                Ok((bytes, sha256, mode)) => {
                    state.commit_file(bytes);
                    files.push(LiveFile {
                        path: relative,
                        mode,
                        bytes,
                        sha256,
                    });
                }
                Err(error) => {
                    if let Err(stop) = record(&mut state, &relative, error) {
                        match stop {
                            DeltaError::TooManyFiles
                            | DeltaError::TooLarge
                            | DeltaError::DeadlineExceeded => {
                                aborted = Some(stop);
                                break;
                            }
                            other => return Err(other),
                        }
                    }
                }
            }
        } else if st.is_symlink() {
            match hash_symlink(
                dir_fd,
                OsStr::new(&name),
                &relative,
                cancellation,
                deadline,
                &mut state,
            ) {
                Ok((bytes, sha256)) => files.push(LiveFile {
                    path: relative,
                    mode: SnapshotFileMode::Symlink,
                    bytes,
                    sha256,
                }),
                Err(error) => {
                    if let Err(stop) = record(&mut state, &relative, error) {
                        match stop {
                            DeltaError::TooManyFiles
                            | DeltaError::TooLarge
                            | DeltaError::DeadlineExceeded => {
                                aborted = Some(stop);
                                break;
                            }
                            other => return Err(other),
                        }
                    }
                }
            }
        } else {
            state.not_included(&relative, NotIncludedReason::SpecialFile);
        }
    }

    state.links.validate().map_err(|_| DeltaError::UnsafePath)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(WalkOutcome {
        files,
        not_included: state.not_included,
        not_included_overflow: state.not_included_overflow,
        aborted,
    })
}

/// A directory nesting deeper than this bound is refused as an unsafe path,
/// and it keeps the ancestor directory streams well under the process
/// descriptor limit.
#[cfg(unix)]
const MAX_WALK_DEPTH: usize = 64;

/// Non-Unix hosts have no anchored walk — no descriptor-relative, no-follow
/// opens to lean on — and the coding feature is Unix-only today, so the walk
/// refuses.
#[cfg(not(unix))]
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
