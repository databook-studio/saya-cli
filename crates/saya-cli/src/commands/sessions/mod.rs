//! Shared saved-session operations and the `saya sessions` subcommand.
//!
//! One operation module backs both this CLI subcommand and the session
//! slash forms (a later packet): listing and deletion return typed
//! outcomes a second surface can render or reuse directly, so the two
//! surfaces cannot disagree about what exists, what was deleted, or what
//! was deliberately kept.

mod operations;
pub(crate) mod render;
pub(crate) mod subcommand;

pub(crate) use operations::{delete_saved_session, list_sessions};
pub(crate) use render::{render_delete, render_listing};
pub(crate) use subcommand::run;

/// Every refusal or failure of `saya sessions` is the documented usage
/// class; success is 0.
pub(crate) const EXIT_SESSIONS_ERROR: i32 = 2;

/// What a session delete never touches, printed on every delete that
/// removed the record.
pub(crate) const RETAINED_NOTE: &str =
    "Project preferences and typed database knowledge are stored separately and were not deleted.";

/// Listing walks every page the store reports up to this many rows and
/// says "(more available)" beyond it.
const LISTING_BOUND: usize = 1000;

/// One listed saved session. Sizes come from the store's own capacity
/// accounting; ages are computed at render time from the modification
/// stamp.
pub(crate) struct SessionRow {
    pub(crate) id: String,
    pub(crate) size_bytes: usize,
    pub(crate) status: SessionCapacityStatus,
    pub(crate) modified_unix_ms: u128,
}

/// The typed outcome of listing saved sessions.
pub(crate) struct SessionListing {
    /// Rows newest first, up to [`LISTING_BOUND`].
    pub(crate) rows: Vec<SessionRow>,
    /// Ids whose deletion is pending (tombstoned), sorted.
    pub(crate) pending: Vec<String>,
    /// More sessions exist beyond the listing bound.
    pub(crate) more: bool,
}

/// The typed outcome of a delete attempt. Every variant is renderable by
/// [`render_delete`] and reusable by the slash surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DeleteOutcome {
    /// Every removable path is gone.
    Deleted,
    /// The record is deleted but some paths could not be removed; a retry
    /// converges.
    Pending { remaining: usize },
    /// No saved session carries this id; nothing was created or touched.
    NoSuchSession,
    /// A live process (or this one) holds the session's lock.
    Live { pid: u32 },
    /// The id is not a filename-safe session id.
    InvalidId,
    /// The target is the calling session itself; only the slash surface
    /// passes a current id.
    CurrentSession,
    /// The store or harness refused for a reported reason.
    Failed(String),
}

use saya_store::SessionCapacityStatus;

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
