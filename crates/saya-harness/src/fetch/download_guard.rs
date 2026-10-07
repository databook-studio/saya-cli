//! The create-only pre-flight of a download: refusing a call that cannot
//! publish before any network I/O or budget is spent.
//!
//! A download without a destination digest publishes create-if-absent: when
//! the destination already exists, the publication guard will refuse it — so
//! the refusal happens here, before the URL is ever requested, instead of
//! after a full body has streamed and spent the run's budget. The check is
//! best-effort and never authoritative: the parent listing can be missing,
//! unreadable, or over its entry bound (the call then proceeds and the guard
//! decides at the rename), and the destination can appear, change, or vanish
//! during the stream — the guard at the rename remains the only decision
//! that publishes or refuses on the destination's actual bytes.

use crate::HarnessError;
use crate::workspace::{MAX_LIST_ENTRIES, Workspace};

use super::download_error::DownloadError;

/// Refuses a create-only download whose destination already exists: any
/// entry under its name — file, directory, symlink, or other — is a
/// destination a create-if-absent publication cannot take. A call carrying
/// a destination digest returns at once: its precondition can only be judged
/// by the publication guard, which streams the destination's bytes under the
/// commit gate at the rename.
pub(super) fn refuse_create_over_existing(
    workspace: &Workspace,
    destination: &str,
    expected_destination_sha256: Option<&str>,
) -> Result<(), DownloadError> {
    if expected_destination_sha256.is_some() {
        return Ok(());
    }
    let (parent, name) = match destination.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", destination),
    };
    match workspace.list(parent, MAX_LIST_ENTRIES) {
        Ok(entries) if entries.iter().any(|entry| entry.name == name) => {
            Err(DownloadError::Workspace(HarnessError::AlreadyExists {
                path: destination.to_owned(),
            }))
        }
        // Absent, or the parent cannot be listed (missing, symlinked, over
        // the entry bound): proceed, and let the publication guard decide at
        // the rename.
        _ => Ok(()),
    }
}
