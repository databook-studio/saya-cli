//! Bounded no-follow source-tree copying and digest construction.

use std::{fs, path::Path, time::Instant};

use saya_agent::CancellationToken;
use sha2::{Digest as _, Sha256};

use crate::{
    HarnessError,
    workspace::{EntryKind, Workspace},
};

use super::super::npm::NpmRuntimeError;

struct CopyState<'a> {
    workspace: &'a Workspace,
    destination: &'a Path,
    cancellation: &'a CancellationToken,
    deadline: Instant,
    files: usize,
    entries: usize,
    bytes: u64,
    digest: Sha256,
}

pub(super) fn copy_tree(
    workspace: &Workspace,
    destination: &Path,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<String, NpmRuntimeError> {
    super::check_active(cancellation, deadline)?;
    create_directory(destination)?;
    super::check_active(cancellation, deadline)?;
    let mut state = CopyState {
        workspace,
        destination,
        cancellation,
        deadline,
        files: 0,
        entries: 0,
        bytes: 0,
        digest: Sha256::new(),
    };
    walk(&mut state, "")?;
    Ok(hex(&state.digest.finalize()))
}

fn walk(state: &mut CopyState<'_>, parent: &str) -> Result<(), NpmRuntimeError> {
    super::check_active(state.cancellation, state.deadline)?;
    let listed = state.workspace.list(parent, super::MAX_SOURCE_ENTRIES);
    super::check_active(state.cancellation, state.deadline)?;
    let listed = listed.map_err(map_workspace_error)?;
    // A source tree past the entry ceiling cannot be copied whole; the listing
    // is served short, so the truncation itself is the refusal — silently
    // copying a subset would publish an incomplete project.
    if listed.truncated {
        return Err(NpmRuntimeError::InvalidProject);
    }
    for entry in listed.entries {
        super::check_active(state.cancellation, state.deadline)?;
        state.entries += 1;
        if state.entries > super::MAX_SOURCE_ENTRIES {
            return Err(NpmRuntimeError::InvalidProject);
        }
        let relative = if parent.is_empty() {
            entry.name.clone()
        } else {
            format!("{parent}/{}", entry.name)
        };
        if excluded(&relative) {
            continue;
        }
        match entry.kind {
            EntryKind::Dir => {
                let target = state.destination.join(&relative);
                create_directory(&target)?;
                super::check_active(state.cancellation, state.deadline)?;
                walk(state, &relative)?;
            }
            EntryKind::File => copy_file(state, &relative, entry.size)?,
            EntryKind::Symlink | EntryKind::Other => return Err(NpmRuntimeError::InvalidProject),
        }
    }
    Ok(())
}

fn copy_file(
    state: &mut CopyState<'_>,
    relative: &str,
    listed_size: u64,
) -> Result<(), NpmRuntimeError> {
    super::check_active(state.cancellation, state.deadline)?;
    state.files += 1;
    if state.files > super::MAX_SOURCE_FILES || listed_size > super::MAX_SOURCE_FILE_BYTES {
        return Err(NpmRuntimeError::InvalidProject);
    }
    state.bytes = state
        .bytes
        .checked_add(listed_size)
        .ok_or(NpmRuntimeError::InvalidProject)?;
    if state.bytes > super::MAX_SOURCE_TOTAL_BYTES {
        return Err(NpmRuntimeError::InvalidProject);
    }
    let read = state.workspace.read(relative, super::MAX_SOURCE_FILE_BYTES);
    super::check_active(state.cancellation, state.deadline)?;
    let file = read.map_err(map_workspace_error)?;
    if file.truncated || file.size != listed_size {
        return Err(NpmRuntimeError::SourceChanged);
    }
    let source_path = state.workspace.root().join(relative);
    let metadata =
        fs::symlink_metadata(&source_path).map_err(|_| NpmRuntimeError::SourceChanged)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != file.size {
        return Err(NpmRuntimeError::SourceChanged);
    }
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::MetadataExt as _;
        if metadata.mode() & 0o111 != 0 {
            0o755
        } else {
            0o644
        }
    };
    #[cfg(not(unix))]
    let mode = 0o644;
    let destination = state.destination.join(relative);
    super::check_active(state.cancellation, state.deadline)?;
    write_copy(&destination, &file.bytes, mode)?;
    super::check_active(state.cancellation, state.deadline)?;
    state.digest.update(relative.as_bytes());
    state.digest.update([0]);
    state
        .digest
        .update(if mode == 0o755 { b"0755" } else { b"0644" });
    state.digest.update([0]);
    state.digest.update(file.digest.as_bytes());
    state.digest.update(b"\n");
    Ok(())
}

fn excluded(relative: &str) -> bool {
    relative.split('/').any(|part| {
        part.eq_ignore_ascii_case(".git")
            || part.eq_ignore_ascii_case(".saya")
            || part.eq_ignore_ascii_case("node_modules")
            || part.eq_ignore_ascii_case("dist")
            || part.eq_ignore_ascii_case(".npmrc")
            || part.eq_ignore_ascii_case(".env")
            || part.to_ascii_lowercase().starts_with(".env.")
    })
}

fn create_directory(path: &Path) -> Result<(), NpmRuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut builder = fs::DirBuilder::new();
        builder
            .mode(0o755)
            .create(path)
            .map_err(|_| NpmRuntimeError::Failed)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(NpmRuntimeError::Failed)
    }
}

fn write_copy(path: &Path, bytes: &[u8], mode: u32) -> Result<(), NpmRuntimeError> {
    #[cfg(not(unix))]
    let _ = mode;
    use std::io::Write as _;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(path).map_err(|_| NpmRuntimeError::Failed)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| NpmRuntimeError::Failed)
}

fn map_workspace_error(error: HarnessError) -> NpmRuntimeError {
    match error {
        HarnessError::BoundsExceeded { .. }
        | HarnessError::SymlinkRefused { .. }
        | HarnessError::NotRegularFile { .. }
        | HarnessError::InvalidPath { .. } => NpmRuntimeError::InvalidProject,
        _ => NpmRuntimeError::Failed,
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
