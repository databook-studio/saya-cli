//! The contained single-source read behind the instruction loader: one
//! [`Workspace`] open of the source's own directory and one contained,
//! no-follow read of the fixed file name, mapped entirely to data.

use std::path::Path;

use crate::HarnessError;
use crate::workspace::{ReadFile, Workspace};

use super::{InstructionOrigin, InstructionSource, SkipReason, SkippedSource};

/// Reads one named file through a contained [`Workspace`] open of `dir`,
/// pushing either a source or a skip and returning the admitted body's byte
/// length for the aggregate cap.
pub(super) fn read_one(
    dir: &Path,
    origin: InstructionOrigin,
    file_name: &'static str,
    max_bytes: usize,
    sources: &mut Vec<InstructionSource>,
    skipped: &mut Vec<SkippedSource>,
) -> usize {
    let workspace = match Workspace::open(dir) {
        Ok(workspace) => workspace,
        Err(_) => {
            skipped.push(SkippedSource {
                origin,
                reason: SkipReason::Unreadable,
            });
            return 0;
        }
    };
    let read = match workspace.read(file_name, max_bytes as u64) {
        Ok(read) => read,
        Err(error) => return skip_error(&error, origin, skipped),
    };
    let Some(body) = decode(&read) else {
        skipped.push(SkippedSource {
            origin,
            reason: SkipReason::NotUtf8,
        });
        return 0;
    };
    let admitted = body.len();
    sources.push(InstructionSource {
        origin,
        file_name,
        body,
        size: read.size,
        sha256: read.digest,
        truncated: read.truncated,
    });
    admitted
}

/// Maps a contained-read error to a skip; absence is silent.
fn skip_error(
    error: &HarnessError,
    origin: InstructionOrigin,
    skipped: &mut Vec<SkippedSource>,
) -> usize {
    if !error.is_not_found() {
        let reason = match error {
            HarnessError::SymlinkRefused { .. } => SkipReason::SymlinkRefused,
            HarnessError::NotRegularFile { .. } => SkipReason::NotRegularFile,
            HarnessError::BoundsExceeded { .. } => SkipReason::TooLarge,
            _ => SkipReason::Unreadable,
        };
        skipped.push(SkippedSource { origin, reason });
    }
    0
}

/// Decodes the admitted bytes. A cap cut mid-character (incomplete tail on a
/// truncated read) keeps the valid prefix; any other defect refuses the
/// whole source.
fn decode(read: &ReadFile) -> Option<String> {
    match std::str::from_utf8(&read.bytes) {
        Ok(body) => Some(body.to_owned()),
        Err(error) if error.error_len().is_none() && read.truncated => {
            let valid = &read.bytes[..error.valid_up_to()];
            Some(String::from_utf8_lossy(valid).into_owned())
        }
        Err(_) => None,
    }
}
