//! The shape one run takes over a session's folder.
//!
//! A session opens over the folder it was started in. Before it prepares a
//! run it asks one question: may this run write that folder? The answer is a
//! plain fact about the folder and its lease — never a rendering, and never a
//! second opinion on what a run's write scope is.
//!
//! - [`WorkspaceShape::Writable`]: nothing holds the folder's lease, so a
//!   write-scoped run may take it.
//! - [`WorkspaceShape::ReadOnlyFiles`]: another live owner holds the lease, so
//!   this run reads the folder's files and stages its own state under the
//!   state root.
//! - [`WorkspaceShape::NoWorkspace`]: the folder cannot be a root at all.

use crate::commands::coding::{folder_lease_held, resolve_in_place};
use std::{fs, path::Path};

/// What a run may do with the folder its session opened in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceShape {
    /// No live owner holds the folder's lease: a run may take it and write.
    Writable,
    /// The folder's files are readable but not writable, because another live
    /// owner holds the lease.
    ReadOnlyFiles {
        /// Why, in the operator's terms.
        reason: String,
    },
    /// The folder cannot be a run's root.
    NoWorkspace {
        /// Why, in the operator's terms.
        reason: String,
    },
}

/// The shape of a run over `folder`, whose own records live under `state_root`.
pub(crate) fn decide(folder: &Path, state_root: &Path) -> WorkspaceShape {
    let Ok(canonical) = fs::canonicalize(folder) else {
        return WorkspaceShape::NoWorkspace {
            reason: "the folder does not exist".to_owned(),
        };
    };
    // The same refusals an in-place root answers to: the filesystem root, the
    // home directory, saya's state directory, and every shape the root's
    // identity will not take. Each states its own reason.
    let root = match resolve_in_place(folder, &canonical, state_root) {
        Ok(root) => root,
        // Every refusal is a fact about the folder, so the refusal is the
        // reason.
        Err(error) => {
            return WorkspaceShape::NoWorkspace {
                reason: error.to_string(),
            };
        }
    };
    match folder_lease_held(state_root, root.id().as_str()) {
        Ok(false) => WorkspaceShape::Writable,
        Ok(true) => WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned(),
        },
        // A lease state this session cannot read is not a folder to write
        // into: the run reads it, as it does one it knows another session
        // holds.
        Err(error) => WorkspaceShape::ReadOnlyFiles {
            reason: format!("another session may own this folder: {error}"),
        },
    }
}
