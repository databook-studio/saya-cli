//! The explicit bounds of one download, and their provisional defaults.
//!
//! A run states them once with its scope; overrunning any of them is a typed
//! error, never a silent short file. The defaults are provisional until M5
//! measures real runs (U8).

use std::time::Duration;

/// Provisional per-file byte bound: large enough for a corpus artifact,
/// small enough that one URL cannot silently fill a disk. Provisional until
/// M5 measures real runs (U8).
pub const DEFAULT_MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Provisional per-request wall clock, matching the `[ai] timeout` default.
/// With resumable partials a longer transfer is a sequence of budgeted
/// attempts, not one unbounded one. Provisional until M5 measures real runs
/// (U8).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// The redirect bound, matching the fetch tool: room for a legitimate
/// chain, cheap to trip on a loop.
pub(super) const MAX_REDIRECT_HOPS: usize = 5;

/// The bounds of one download, stated rather than implied.
#[derive(Clone, Copy, Debug)]
pub struct DownloadLimits {
    /// Maximum bytes of one downloaded file. Trips typed, before the byte
    /// that would exceed it is written.
    pub max_file_bytes: u64,
    /// Wall-clock budget for one request — DNS, the GET, and its body read.
    pub request_timeout: Duration,
    /// Maximum redirects followed, each re-judged by the policy.
    pub max_redirect_hops: usize,
}

impl Default for DownloadLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_redirect_hops: MAX_REDIRECT_HOPS,
        }
    }
}
