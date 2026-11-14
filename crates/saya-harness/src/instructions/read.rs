//! The contained single-source read behind the instruction loader: one
//! contained, no-follow read of a workspace-relative path, mapped entirely to
//! data.

use crate::HarnessError;
use crate::workspace::{ReadFile, Workspace};

use super::{InstructionOrigin, InstructionSource, SkipReason, SkippedSource};

/// What one named file contributed.
pub(super) enum ReadOutcome {
    /// The file does not exist: silent, and the caller may try a fallback.
    Absent,
    /// The file was admitted; the payload is its admitted byte length.
    Admitted(usize),
    /// The file exists but contributed nothing; the skip is already recorded.
    Skipped,
}

/// Reads one workspace-relative file through a contained [`Workspace`],
/// pushing either a source or a skip and returning the admitted body's byte
/// length for the aggregate cap. A missing file is [`ReadOutcome::Absent`]:
/// the caller decides whether a fallback name is tried.
pub(super) fn read_one(
    workspace: &Workspace,
    path: &str,
    origin: InstructionOrigin,
    file_name: &'static str,
    max_bytes: usize,
    sources: &mut Vec<InstructionSource>,
    skipped: &mut Vec<SkippedSource>,
) -> ReadOutcome {
    let read = match workspace.read(path, max_bytes as u64) {
        Ok(read) => read,
        Err(error) if error.is_not_found() => return ReadOutcome::Absent,
        Err(error) => {
            skipped.push(SkippedSource {
                origin,
                file_name,
                reason: skip_reason(&error),
            });
            return ReadOutcome::Skipped;
        }
    };
    let Some(body) = decode(&read) else {
        skipped.push(SkippedSource {
            origin,
            file_name,
            reason: SkipReason::NotUtf8,
        });
        return ReadOutcome::Skipped;
    };
    let admitted = body.len();
    sources.push(InstructionSource {
        origin,
        file_name,
        path: path.to_owned(),
        body,
        size: read.size,
        sha256: read.digest,
        truncated: read.truncated,
    });
    ReadOutcome::Admitted(admitted)
}

/// Maps a contained-read error to its typed skip reason.
pub(super) fn skip_reason(error: &HarnessError) -> SkipReason {
    match error {
        HarnessError::SymlinkRefused { .. } => SkipReason::SymlinkRefused,
        HarnessError::NotRegularFile { .. } => SkipReason::NotRegularFile,
        HarnessError::BoundsExceeded { .. } => SkipReason::TooLarge,
        _ => SkipReason::Unreadable,
    }
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
