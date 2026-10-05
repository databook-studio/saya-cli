//! Session rotation: publishing an explicit successor record that links to
//! its parent — the mechanics behind "continue in a fresh session". Nothing
//! rotates automatically; grants and approvals are never transferred; the
//! parent record is never written; and selection — which session becomes
//! active — is the caller's job after [`SuccessorOutcome`] returns. The
//! publication is the ordinary atomic save, so a failure leaves the parent
//! fully usable with either no successor or a complete one.

use crate::filesystem::{FsSessionStore, is_valid_id};
use crate::private_file::bounded_read;
use crate::replace::{AtomicReplace, Replacer};
use crate::session_lifecycle::{SessionCapacityStatus, tombstone};
use crate::{MAX_SESSION_BYTES, ParentLink, RedactedSession, StoreError};
use std::{fs, io::ErrorKind, path::Path};

/// The outcome of [`FsSessionStore::publish_successor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessorOutcome {
    /// The successor was published carrying the requested parent link.
    Published,
    /// A successor with exactly this parent link was already published: the
    /// stored record is left byte-identical and the seed is ignored.
    AlreadyPublished,
}

/// The rotation identity shape: at most 64 characters, `[A-Za-z0-9_-]`
/// only — no path metacharacters, bounded length.
fn plain_shape(id: &str) -> bool {
    id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A rotation id: non-empty and in the rotation identity shape.
fn valid_rotation_id(rotation_id: &str) -> bool {
    !rotation_id.is_empty() && plain_shape(rotation_id)
}

/// A rotation participant id: the store's own id rule tightened to the
/// rotation identity shape.
fn valid_store_id(id: &str) -> bool {
    is_valid_id(id) && plain_shape(id)
}

/// Whether `record` carries exactly `link`.
fn link_matches(record: &RedactedSession, link: &ParentLink) -> bool {
    record.parent.as_ref() == Some(link)
}

/// Whether the record at `path` carries exactly `link`, read WITHOUT
/// quarantining — anything unparsable or differently linked is a conflict.
fn existing_link_matches(path: &Path, link: &ParentLink) -> Result<bool, StoreError> {
    let bytes = bounded_read(path, MAX_SESSION_BYTES)?.ok_or(StoreError::Conflict)?;
    let record: RedactedSession =
        serde_json::from_slice(&bytes).map_err(|_| StoreError::Conflict)?;
    Ok(link_matches(&record, link))
}

impl FsSessionStore {
    /// Publishes `successor` as the child of an explicit rotation of the
    /// session `parent_id`, stamped with the [`ParentLink`] for
    /// `(parent_id, rotation_id)`; the parent record is never written.
    /// Repeating the same `(parent, rotation_id)` request converges to
    /// exactly one child: an existing successor whose link matches exactly
    /// returns [`SuccessorOutcome::AlreadyPublished`] byte-identical (the
    /// seed is ignored); any other existing record is a conflict. The seed
    /// never supplies the link — a foreign one is refused; the store writes
    /// the requested link itself. An over-cap seed is refused before
    /// anything is written; a warning capacity is allowed and the caller
    /// decides. Tombstoned ids are refused and the parent must exist and
    /// load.
    pub fn publish_successor(
        &self,
        parent_id: &str,
        rotation_id: &str,
        successor: RedactedSession,
    ) -> Result<SuccessorOutcome, StoreError> {
        self.publish_successor_inner(parent_id, rotation_id, successor, &AtomicReplace)
    }

    /// Test-only publish through an injected [`Replacer`]: proves a failed
    /// publication leaves the parent usable and no successor behind, on any
    /// platform, the same seam the plain save tests use.
    #[cfg(test)]
    pub(crate) fn publish_successor_with_replacer(
        &self,
        parent_id: &str,
        rotation_id: &str,
        successor: RedactedSession,
        replacer: &dyn Replacer,
    ) -> Result<SuccessorOutcome, StoreError> {
        self.publish_successor_inner(parent_id, rotation_id, successor, replacer)
    }

    fn publish_successor_inner(
        &self,
        parent_id: &str,
        rotation_id: &str,
        mut successor: RedactedSession,
        replacer: &dyn Replacer,
    ) -> Result<SuccessorOutcome, StoreError> {
        if !valid_store_id(parent_id)
            || !valid_store_id(&successor.id)
            || !valid_rotation_id(rotation_id)
            || parent_id == successor.id
        {
            return Err(StoreError::Invalid);
        }
        if tombstone::exists(self.sessions_root(), parent_id)?
            || tombstone::exists(self.sessions_root(), &successor.id)?
        {
            return Err(StoreError::Conflict);
        }
        // The parent must exist and load; a corrupt parent is not usable.
        // Its content is never read into the successor and never written.
        self.load_file(&self.path(parent_id)?)?
            .ok_or(StoreError::NotFound)?;
        let successor_path = self.path(&successor.id)?;
        let requested = ParentLink {
            id: parent_id.to_owned(),
            rotation_id: rotation_id.to_owned(),
        };
        match fs::symlink_metadata(&successor_path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(StoreError::unavailable()),
            Ok(_) => {
                return if existing_link_matches(&successor_path, &requested)? {
                    Ok(SuccessorOutcome::AlreadyPublished)
                } else {
                    Err(StoreError::Conflict)
                };
            }
        }
        // The seed never supplies the link: a foreign one is refused; the
        // store writes the requested link itself.
        if successor.parent.is_some() && !link_matches(&successor, &requested) {
            return Err(StoreError::Invalid);
        }
        successor.parent = Some(requested);
        // Capacity with the link included, the save's exact accounting.
        if self.capacity_of(successor.clone())?.status == SessionCapacityStatus::OverCap {
            return Err(StoreError::LimitExceeded);
        }
        self.save_inner(successor, replacer)?;
        Ok(SuccessorOutcome::Published)
    }
}
