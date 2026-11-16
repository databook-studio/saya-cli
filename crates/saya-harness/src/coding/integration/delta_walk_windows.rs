//! The Windows live-checkout walk. Windows has no `openat`, so the walk is
//! path-based: every directory is validated by the contained workspace walk,
//! every file is opened through it (reparse refusal plus a post-open identity
//! check), and every reparse point that is not a readable symlink is reported
//! rather than followed. Bounds, ignore rules, and the not-included report
//! match the unix walk.

use std::{
    collections::HashSet,
    fs,
    io::{ErrorKind, Read},
    path::Path,
    time::Instant,
};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

use super::super::delta_types::{
    DeltaError, LiveFile, MAX_DELTA_NOT_INCLUDED, NotIncludedPath, NotIncludedReason, WalkOutcome,
};
use super::MAX_WALK_DEPTH;
use crate::coding::git::IgnoredPaths;
use crate::coding::snapshot::{
    MAX_SNAPSHOT_FILE_BYTES, MAX_SNAPSHOT_FILES, MAX_SNAPSHOT_TOTAL_BYTES,
    MAX_SYMLINK_TARGET_BYTES, SnapshotFileMode, SnapshotPath, SymlinkSet, validate_symlink_target,
};
use crate::workspace::{Workspace, windows_identity};

#[derive(Default)]
struct WalkState {
    total_bytes: u64,
    file_count: usize,
    links: SymlinkSet,
    not_included: Vec<NotIncludedPath>,
    not_included_overflow: usize,
}

impl WalkState {
    fn not_included(&mut self, path: &str, reason: NotIncludedReason) {
        if self.not_included.len() < MAX_DELTA_NOT_INCLUDED {
            self.not_included
                .push(NotIncludedPath::new(path.to_owned(), reason));
        } else {
            self.not_included_overflow = self.not_included_overflow.saturating_add(1);
        }
    }

    fn commit_file(&mut self, bytes: u64) {
        self.file_count = self.file_count.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(bytes);
    }
}

/// Walks the checkout rooted at `root` and returns every live file it could
/// include, sorted by path. Every directory is validated by the contained
/// workspace walk; every file is opened through it and re-verified against
/// the scanned identity. Descent is depth-first with one directory handle per
/// ancestor, and the root-level `.saya` directory is bounded and counted but
/// never classified or digested.
pub(crate) fn walk_checkout_with_root_git_directory(
    root: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
    allow_root_git_directory: bool,
    omitted: &HashSet<String>,
    ignored: &IgnoredPaths,
) -> Result<WalkOutcome, DeltaError> {
    let workspace = Workspace::open(root).map_err(|_| DeltaError::Io)?;
    let root_dir = fs::read_dir(workspace.root()).map_err(|_| DeltaError::Io)?;
    let mut stack: Vec<(String, fs::ReadDir)> = vec![(String::new(), root_dir)];
    let mut files: Vec<LiveFile> = Vec::new();
    let mut state = WalkState::default();
    let mut aborted = None;

    while let Some((prefix, stream)) = stack.last_mut() {
        if let Err(error) = check_gate(cancellation, deadline) {
            if error == DeltaError::Cancelled {
                return Err(error);
            }
            aborted = Some(error);
            break;
        }
        let Some(entry) = stream.next() else {
            stack.pop();
            continue;
        };
        let entry = entry.map_err(|_| DeltaError::Io)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DeltaError::UnsafePath)?;
        let is_root = prefix.is_empty();
        let relative = if is_root {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let file_type = entry.file_type().map_err(|_| DeltaError::Io)?;
        let reparse = windows_identity::path_is_reparse_point(&path).map_err(|_| DeltaError::Io)?;
        // The root `.saya` output root is scanned and bounded even though the
        // baseline omits it, so scratch growth cannot go unnoticed.
        if is_root
            && name == super::super::delta_output::OUTPUT_ROOT_NAME
            && file_type.is_dir()
            && !reparse
        {
            super::super::delta_output::scan_output_root_windows(&path, cancellation, deadline)?;
            continue;
        }
        // The baseline's own omissions are not job content.
        if omitted.contains(relative.as_str()) {
            continue;
        }
        // The checkout's own ignore rules decide what is job content.
        if ignored.covers(Path::new(&relative)) {
            continue;
        }
        if name.eq_ignore_ascii_case(".git") {
            let is_metadata = if is_root && name == ".git" {
                file_type.is_file() || allow_root_git_directory && file_type.is_dir()
            } else {
                false
            };
            if is_metadata {
                continue;
            }
            state.not_included(&relative, NotIncludedReason::NestedRepository);
            continue;
        }
        if SnapshotPath::parse(relative.clone()).is_err() {
            state.not_included(&relative, NotIncludedReason::UnsafePath);
            continue;
        }
        if file_type.is_dir() && !reparse {
            if stack.len() >= MAX_WALK_DEPTH {
                state.not_included(&relative, NotIncludedReason::UnsafePath);
                continue;
            }
            let child = match fs::read_dir(&path) {
                Ok(child) => child,
                Err(_) => return Err(DeltaError::Io),
            };
            stack.push((relative, child));
        } else if file_type.is_symlink() {
            match hash_symlink(&path, &relative, cancellation, deadline, &mut state) {
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
        } else if file_type.is_file() && !reparse {
            match hash_file(&workspace, &path, &relative, cancellation, deadline, &state) {
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

fn record(state: &mut WalkState, path: &str, error: DeltaError) -> Result<(), DeltaError> {
    match error {
        DeltaError::FileTooLarge => state.not_included(path, NotIncludedReason::Oversize),
        DeltaError::Symlink => state.not_included(path, NotIncludedReason::SpecialFile),
        DeltaError::UnsafePath => state.not_included(path, NotIncludedReason::UnsafePath),
        other => return Err(other),
    }
    Ok(())
}

fn hash_file(
    workspace: &Workspace,
    path: &Path,
    rel: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
    state: &WalkState,
) -> Result<(u64, Sha256Digest, SnapshotFileMode), DeltaError> {
    if state.file_count >= MAX_SNAPSHOT_FILES {
        return Err(DeltaError::TooManyFiles);
    }
    let pre = fs::symlink_metadata(path).map_err(|_| DeltaError::Io)?;
    if pre.file_type().is_symlink() {
        return Err(DeltaError::Symlink);
    }
    if !pre.is_file() {
        return Err(DeltaError::Symlink);
    }
    if pre.len() > MAX_SNAPSHOT_FILE_BYTES {
        return Err(DeltaError::FileTooLarge);
    }
    let mut file = workspace
        .open_verified(path, &pre, rel)
        .map_err(|_| DeltaError::Io)?;
    let meta = file.metadata().map_err(|_| DeltaError::Io)?;
    if !meta.is_file() {
        return Err(DeltaError::Symlink);
    }
    let limit = MAX_SNAPSHOT_FILE_BYTES.saturating_add(1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_CHUNK_BYTES];
    let mut read_total: u64 = 0;
    loop {
        check_gate(cancellation, deadline)?;
        let remaining = limit - read_total;
        if remaining == 0 {
            return Err(DeltaError::FileTooLarge);
        }
        let want = remaining.min(READ_CHUNK_BYTES as u64) as usize;
        match file.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => {
                hasher.update(&buffer[..read]);
                read_total += read as u64;
                let total = state
                    .total_bytes
                    .checked_add(read_total)
                    .ok_or(DeltaError::TooLarge)?;
                if total > MAX_SNAPSHOT_TOTAL_BYTES {
                    return Err(DeltaError::TooLarge);
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(DeltaError::Io),
        }
    }
    check_gate(cancellation, deadline)?;
    let sha256 = Sha256Digest::parse(&hex_lower(&hasher.finalize())).map_err(|_| DeltaError::Io)?;
    Ok((read_total, sha256, SnapshotFileMode::Regular))
}

fn hash_symlink(
    path: &Path,
    rel: &str,
    cancellation: &CancellationToken,
    deadline: Instant,
    state: &mut WalkState,
) -> Result<(u64, Sha256Digest), DeltaError> {
    if state.file_count >= MAX_SNAPSHOT_FILES {
        return Err(DeltaError::TooManyFiles);
    }
    check_gate(cancellation, deadline)?;
    let target = fs::read_link(path).map_err(|_| DeltaError::Io)?;
    let target = target.to_string_lossy().replace('\\', "/").into_bytes();
    if target.len() > MAX_SYMLINK_TARGET_BYTES {
        return Err(DeltaError::FileTooLarge);
    }
    validate_symlink_target(rel, &target).map_err(|_| DeltaError::UnsafePath)?;
    let bytes = target.len() as u64;
    let total = state
        .total_bytes
        .checked_add(bytes)
        .ok_or(DeltaError::TooLarge)?;
    if total > MAX_SNAPSHOT_TOTAL_BYTES {
        return Err(DeltaError::TooLarge);
    }
    check_gate(cancellation, deadline)?;
    state.links.insert(rel, target.clone());
    state.commit_file(bytes);
    let sha256 =
        Sha256Digest::parse(&hex_lower(&Sha256::digest(&target))).map_err(|_| DeltaError::Io)?;
    Ok((bytes, sha256))
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

fn hex_lower(bytes: &[u8]) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    hex
}

const READ_CHUNK_BYTES: usize = 64 * 1024;
