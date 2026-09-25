//! The typed failures of a download and what a completed one reports.

use std::time::Duration;

use super::policy::FetchRefusal;
use super::transport::FetchTransportError;
use crate::HarnessError;

/// What a completed download reports: the contained destination it landed
/// at, its byte length, and the SHA-256 of the whole content — the digest a
/// resume verifies against, and what a caller's spec can pin.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    /// The workspace-relative destination the file landed at.
    pub destination: String,
    /// Bytes written.
    pub bytes: u64,
    /// Lowercase-hex SHA-256 of the complete file.
    pub sha256: String,
}

/// Why a download failed. Every bound overrun is typed; a partial is never
/// passed off as the whole file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DownloadError {
    /// The URL or a redirect target was refused by the policy, or a
    /// resolved address was refused before connecting.
    #[error("download refused: {0}")]
    Refused(#[from] FetchRefusal),

    /// More redirects were followed than `max_redirect_hops` allows.
    #[error("download exceeded {limit} redirects")]
    TooManyRedirects { limit: usize },

    /// The response was not a success status (including a 416 to a range).
    #[error("download of {url} returned status {status}")]
    UnsuccessfulStatus { url: String, status: u16 },

    /// A 206 did not serve the offset the resume asked for — the server's
    /// view of the resource differs from the recorded partial.
    #[error("resume of {url} refused: server served {served}, not the requested offset {expected}")]
    RangeMismatch {
        url: String,
        expected: u64,
        served: String,
    },

    /// The file exceeded `max_file_bytes`. Trips before the overrunning
    /// byte is written; the partial below the bound is left in place.
    #[error("download file exceeded {limit} bytes")]
    FileTooLarge { limit: u64 },

    /// The run's download budget tripped — paused, not overrun: the byte
    /// that would exceed it was never written, and the partial is left
    /// resumable. The engine surfaces this as `PauseReason::BudgetExhausted`.
    #[error("the run's download budget of {limit} bytes tripped after {received}")]
    BudgetExhausted { limit: u64, received: u64 },

    /// One request — DNS, GET, or body read — did not finish inside its
    /// wall-clock budget; the partial is left resumable.
    #[error("download exceeded its {budget:.0?} wall-clock budget")]
    DeadlineExceeded { budget: Duration },

    /// A resumed download's recorded state does not match the disk: a
    /// digest, length, or server-offset mismatch. The partial is left in
    /// place as evidence, never silently restarted over.
    #[error("resume of {path} failed: {detail}")]
    ResumeMismatch { path: String, detail: String },

    /// The transport failed — an unresolvable name, a broken connection.
    #[error("download transport failed: {0}")]
    Transport(FetchTransportError),

    /// Containment refused the destination, or the workspace I/O failed.
    #[error("download workspace error: {0}")]
    Workspace(#[from] HarnessError),
}
