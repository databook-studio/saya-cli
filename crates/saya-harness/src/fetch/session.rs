//! The streaming phase of one download attempt: the open partial under
//! every bound.
//!
//! [`Session`] is the contained part file plus the continuous digest state;
//! [`stream_body`] pulls the body chunk by chunk — never buffered whole —
//! under the per-file bound, the run's download budget (claimed before the
//! byte is written: paused, not overrun), and the per-request wall clock.
//! Every error path persists the sidecar best-effort, so the partial on disk
//! stays resumable no matter how the attempt ends; a stale sidecar is always
//! safe, because a resume truncates the unaccounted tail.

use std::io::Write;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tokio::time::Instant;

use super::budget::DownloadBudget;
use super::download::under_deadline;
use super::download_error::DownloadError;
use super::download_limits::DownloadLimits;
use super::partial::{self, PartialMeta};
use super::transport::WireResponse;
use crate::{io_error, workspace::Workspace};

/// The open partial: the contained part file plus the continuous digest.
pub(super) struct Session {
    pub(super) path: PathBuf,
    pub(super) file: std::fs::File,
    pub(super) hasher: Sha256,
    pub(super) total: u64,
}

impl Session {
    /// Persists the resume contract: exactly `total` bytes are on disk and
    /// digested. Best-effort on error paths — a stale sidecar is always safe
    /// (a resume truncates the unaccounted tail), so the original typed
    /// failure is what surfaces.
    pub(super) fn persist(
        &self,
        workspace: &Workspace,
        destination: &str,
        url: &str,
    ) -> Result<(), DownloadError> {
        partial::write_meta(
            workspace,
            destination,
            &PartialMeta {
                url: url.to_owned(),
                len: self.total,
                sha256: partial::hex(self.hasher.clone().finalize().as_slice()),
            },
        )
    }
}

pub(super) fn fresh(
    workspace: &Workspace,
    destination: &str,
    url: &str,
) -> Result<Session, DownloadError> {
    let open = partial::start_fresh(workspace, destination, url)?;
    Ok(Session {
        path: open.path,
        file: open.file,
        hasher: open.hasher,
        total: 0,
    })
}

/// Streams one response body into the open partial under every bound. Each
/// error path persists the sidecar best-effort, so the partial on disk stays
/// resumable no matter how the attempt ends.
pub(super) async fn stream_body(
    session: &mut Session,
    response: WireResponse,
    workspace: &Workspace,
    destination: &str,
    url: &str,
    limits: DownloadLimits,
    budget: &DownloadBudget,
) -> Result<(), DownloadError> {
    let deadline = Instant::now() + limits.request_timeout;
    let mut body = response.body;
    loop {
        let chunk = match under_deadline(limits.request_timeout, deadline, body.next_chunk()).await
        {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = session.persist(workspace, destination, url);
                return Err(error);
            }
        };
        let Some(chunk) = chunk else {
            return Ok(());
        };
        if session.total.saturating_add(chunk.len() as u64) > limits.max_file_bytes {
            let _ = session.persist(workspace, destination, url);
            return Err(DownloadError::FileTooLarge {
                limit: limits.max_file_bytes,
            });
        }
        if !budget.claim(chunk.len() as u64) {
            let _ = session.persist(workspace, destination, url);
            return Err(DownloadError::BudgetExhausted {
                limit: budget.limit(),
                received: session.total,
            });
        }
        // A bounded chunk write to the page cache — the byte bounds above
        // cap the total; no whole body is ever buffered.
        if let Err(error) = session.file.write_all(&chunk) {
            let _ = session.persist(workspace, destination, url);
            return Err(io_error("write download chunk", &session.path, error).into());
        }
        session.hasher.update(&chunk);
        session.total += chunk.len() as u64;
    }
}
