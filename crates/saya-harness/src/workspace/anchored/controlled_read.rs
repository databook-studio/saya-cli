use saya_agent::CancellationToken;
use sha2::{Digest, Sha256};
use std::io::Read as _;

use super::super::contain::{ControlledReadError, ReadFile, Workspace};
use super::final_stat;
use crate::{HarnessError, io_error};

pub(crate) fn read(
    ws: &Workspace,
    rel: &str,
    max_bytes: u64,
    file_limit: u64,
    cancellation: &CancellationToken,
) -> Result<ReadFile, ControlledReadError> {
    use ControlledReadError::{Cancelled, Workspace as WorkspaceError};

    if cancellation.is_cancelled() {
        return Err(Cancelled);
    }
    let anchor = ws.anchor(rel, false).map_err(WorkspaceError)?;
    let stat = final_stat(&anchor, "read workspace file", rel).map_err(WorkspaceError)?;
    if stat.is_symlink() {
        return Err(WorkspaceError(HarnessError::SymlinkRefused {
            path: rel.to_string(),
        }));
    }
    if !stat.is_file() {
        return Err(WorkspaceError(HarnessError::NotRegularFile {
            path: rel.to_string(),
        }));
    }
    if stat.len() > file_limit {
        return Err(WorkspaceError(HarnessError::BoundsExceeded {
            path: rel.to_string(),
            found: stat.len(),
            max: file_limit,
        }));
    }
    let file = anchor
        .open_verified(stat.identity(), rel)
        .map_err(WorkspaceError)?;
    let mut bytes = Vec::new();
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut kept = 0u64;
    let mut scanned = 0u64;
    loop {
        if cancellation.is_cancelled() {
            return Err(Cancelled);
        }
        let read = (&file).read(&mut chunk).map_err(|error| {
            WorkspaceError(io_error("read workspace file", anchor.path(), error))
        })?;
        if read == 0 {
            break;
        }
        scanned += read as u64;
        if scanned > file_limit {
            return Err(WorkspaceError(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: scanned,
                max: file_limit,
            }));
        }
        hasher.update(&chunk[..read]);
        let take = read.min(max_bytes.saturating_sub(kept) as usize);
        bytes.extend_from_slice(&chunk[..take]);
        kept += take as u64;
    }
    if cancellation.is_cancelled() {
        return Err(Cancelled);
    }
    Ok(ReadFile {
        bytes,
        size: scanned,
        truncated: scanned > max_bytes,
        digest: hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    })
}
