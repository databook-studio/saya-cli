//! The capture reader's typed refusals: mapping the anchored walk's and the
//! verified open's `HarnessError`s onto `SnapshotError` capture variants,
//! and building the bounded `CaptureIo` descriptions. The failing path a
//! harness error carries is dropped, never rendered: Display bodies stay
//! bounded and path-free.

use crate::HarnessError;

use super::{RedactedIoError, SnapshotError};

/// Maps a refusal from the anchored walk or the verified open onto the typed
/// capture refusal. The stage names the failing step for `CaptureIo` only.
pub(super) fn refuse(stage: &'static str, error: HarnessError) -> SnapshotError {
    match error {
        HarnessError::NotFound { .. } => SnapshotError::SourceDisappeared,
        HarnessError::SymlinkRefused { .. } => SnapshotError::SourceSymlink,
        HarnessError::NotRegularFile { .. } => SnapshotError::SourceNotRegularFile,
        HarnessError::IdentityChanged { .. } => SnapshotError::SourceIdentityChanged,
        HarnessError::Io { source, .. } => source_io(stage, source),
        // The plan path was validated at plan time, so the residual walk
        // refusals name a plan this walk can no longer reproduce.
        _ => SnapshotError::UnsafePath,
    }
}

/// Maps a raw io error from a source-side stage — the scan, the verified
/// open, or the post-read final stat — onto the typed capture refusal. A
/// NotFound means the planned source vanished mid-capture: it reports as
/// `SourceDisappeared`, never as a generic capture failure. Sink write
/// failures keep `capture_io`; there the source did not disappear.
pub(super) fn source_io(stage: &'static str, error: std::io::Error) -> SnapshotError {
    if error.kind() == std::io::ErrorKind::NotFound {
        SnapshotError::SourceDisappeared
    } else {
        capture_io(stage, error)
    }
}

/// A bounded, path-free description for a capture I/O failure: the stage
/// name plus the io error kind, truncated to the cap. The original error
/// rides as the redacted cause for programmatic consumers; the Display shows
/// only the bounded description.
pub(super) fn capture_io(stage: &'static str, source: std::io::Error) -> SnapshotError {
    const MAX_DESCRIPTION_CHARS: usize = 128;
    let description: String = format!("{stage}: {}", source.kind())
        .chars()
        .take(MAX_DESCRIPTION_CHARS)
        .collect();
    SnapshotError::CaptureIo {
        description,
        source: Some(RedactedIoError::new(source)),
    }
}
