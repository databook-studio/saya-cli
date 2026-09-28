//! The saved-investigation document repository (D2): one portable JSON
//! document per investigation id at `<root>/<id>.json`, plus a per-machine
//! local binding at `<root>/local/<id>.json` that never leaves the machine.
//!
//! Every mutation runs entirely under the cross-process repository lock
//! (the `lock` submodule) — read/check/publish is one critical section —
//! and create's publish never replaces, so a document that appears after the checks is a
//! conflict, never an overwrite. Every write stages private temp bytes
//! beside its target and publishes with an atomic rename; every read is
//! bounded and never quarantines, rewrites, or deletes a file it cannot
//! parse. A corrupt document is reported, so a bad file can never be
//! silently promoted to good.

mod binding;
mod documents;
mod list;
mod lock;
mod stale;

#[cfg(test)]
mod exclusivity_tests;

#[cfg(test)]
mod lock_protocol_tests;

#[cfg(test)]
mod tests;

use crate::{StoreError, private_file};
use saya_types::investigation::InvestigationId;
use std::path::PathBuf;
use std::time::Duration;

pub use binding::{LocalBinding, MAX_BINDING_BYTES};
pub use list::{InvestigationListIssue, InvestigationPage, InvestigationSummary, MAX_LIST_PAGE};

/// The most documents one collection may hold (D2). The cap is checked
/// against the same candidate predicate the list scan uses, so corrupt or
/// unreadable documents still occupy slots and cannot be used to bypass it.
pub const MAX_DOCUMENTS: usize = 500;

/// The most candidate documents one scan will consider: the collection cap
/// plus one, so a collection that somehow exceeds the cap is still detected
/// and surfaced through the page's `capped` flag.
pub(crate) const LIST_SCAN_BOUND: usize = MAX_DOCUMENTS + 1;

/// Bounded, atomic, revision-checked storage for saved-investigation
/// documents. Cheap to construct and clone; every method takes `&self`.
pub struct InvestigationRepository {
    root: PathBuf,
    lock_wait: Duration,
    stale_after: Duration,
}

impl InvestigationRepository {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock_wait: lock::LOCK_WAIT,
            stale_after: lock::STALE_AFTER,
        }
    }

    /// The document path for `id`. The id is re-checked here — never
    /// trusted to have been validated upstream — so an unvalidated string
    /// can never become a path component (`..`, a separator, a `local/`
    /// escape).
    pub(crate) fn document_path(&self, id: &str) -> Result<PathBuf, StoreError> {
        if InvestigationId::parse(id).is_err() {
            return Err(StoreError::Invalid);
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    /// Creates the root (0700 on unix) if missing.
    pub(crate) fn ensure_root(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.root).map_err(private_file::io_error)?;
        #[cfg(unix)]
        private_file::set_mode(&self.root, 0o700)?;
        Ok(())
    }

    /// Scans the root for candidate documents: regular files named
    /// `<id>.json` whose stem parses as an [`InvestigationId`]. Staging temp
    /// files (dot-prefixed, `.tmp`-suffixed), the `local/` binding
    /// directory, and anything else are ignored. The scan stops at
    /// [`LIST_SCAN_BOUND`] candidates, which is enough to enforce the
    /// collection cap and to know that more exist.
    pub(crate) fn scan_candidates(&self) -> Result<Vec<InvestigationId>, StoreError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(StoreError::unavailable()),
        };
        let mut candidates = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            if candidates.len() >= LIST_SCAN_BOUND {
                break;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if let Ok(id) = InvestigationId::parse(stem) {
                candidates.push(id);
            }
        }
        candidates.sort();
        Ok(candidates)
    }
}

/// Maps a definition-document failure onto the payload-free store error:
/// oversize is a limit, an unknown version is its own refusal, and every
/// other validation or parse failure is an invalid document.
pub(crate) fn store_error_of(error: saya_types::investigation::InvestigationError) -> StoreError {
    match error {
        saya_types::investigation::InvestigationError::Oversize(_) => StoreError::LimitExceeded,
        saya_types::investigation::InvestigationError::UnsupportedVersion(_) => {
            StoreError::VersionUnsupported
        }
        _ => StoreError::Invalid,
    }
}
