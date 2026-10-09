//! One download call, bundled: the record the entry point receives, with
//! the turn's cancellation token riding beside the bounds it shares.

use saya_agent::CancellationToken;

use super::super::budget::DownloadBudget;
use super::super::download_limits::DownloadLimits;
use super::super::policy::FetchPolicy;
use super::super::transport::FetchTransport;
use crate::workspace::Workspace;

/// One download call: everything the entry point needs, bundled so the
/// argument list stays under the lint cap and the turn's token rides beside
/// the bounds it shares.
pub struct DownloadCall<'a> {
    /// The policy that judges the URL and every redirect target.
    pub policy: &'a FetchPolicy,
    /// The wire the request rides.
    pub transport: &'a dyn FetchTransport,
    /// The workspace the download lands in.
    pub workspace: &'a Workspace,
    /// The workspace-relative destination.
    pub destination: &'a str,
    /// The destination precondition, supplied per call: `None` is
    /// create-if-absent — an existing destination refuses before any
    /// network I/O and again at publication; `Some(sha256)` replaces only a
    /// destination whose current bytes hash to it, judged at the rename. A
    /// partial's sidecar proves only the partial's own URL, bytes, and
    /// digest — it never grants replacement authority.
    pub expected_destination_sha256: Option<&'a str>,
    /// The URL to download.
    pub url: &'a str,
    /// The per-attempt and per-file bounds.
    pub limits: DownloadLimits,
    /// The run's shared download wallet.
    pub budget: &'a DownloadBudget,
    /// The turn's cancellation token.
    pub cancellation: &'a CancellationToken,
}
