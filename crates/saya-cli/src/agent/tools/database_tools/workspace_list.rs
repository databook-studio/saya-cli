//! The directory-listing tool: one bounded listing from the run workspace.
//! Path resolution is delegated entirely to `saya_harness::workspace::
//! Workspace::list` — containment, symlink refusal, and the entry bound are
//! the harness's, never re-derived here. An over-bound directory is the
//! harness's typed bounds error, surfaced as-is: a quietly shortened listing
//! would read as the whole directory.

use saya_agent::ToolError;

use super::DatabaseTools;

/// The entry bound passed to the workspace. A directory holding more than
/// this many entries refuses rather than truncating — five hundred named
/// entries already exceeds what a model can reason about in one turn, so a
/// directory that large means the model should narrow its question (list a
/// deeper path) instead of paging through a flood.
pub(crate) const WORKSPACE_LIST_MAX_ENTRIES: usize = 500;

impl DatabaseTools {
    /// Lists one directory from the run workspace under containment. This
    /// tool never touches a connection — a workspace-only run has no
    /// selected profile — so dispatch routes it before connection resolution
    /// (see `dispatch`). The empty path names the workspace root, so `path`
    /// is optional; a present-but-non-string path was already refused at
    /// validation. With no workspace attached it denies with a typed error:
    /// no workspace, no listing.
    pub(super) async fn workspace_list(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let rel = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let entries = workspace
            .list(rel, WORKSPACE_LIST_MAX_ENTRIES)
            .map_err(|error| ToolError::Workspace(error.to_string()))?;
        // `kind` is a machine-readable label the harness's `EntryKind`;
        // `size` is a file's byte length (0 for directories and links).
        let listed: Vec<serde_json::Value> = entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.name,
                    "kind": kind_label(entry.kind),
                    "size": entry.size,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "path": rel,
            "entries": listed,
        }))
    }
}

/// The wire label for one entry kind, stable across the definition's
/// description and any model reading of the result.
fn kind_label(kind: saya_harness::workspace::EntryKind) -> &'static str {
    match kind {
        saya_harness::workspace::EntryKind::File => "file",
        saya_harness::workspace::EntryKind::Dir => "dir",
        saya_harness::workspace::EntryKind::Symlink => "symlink",
        saya_harness::workspace::EntryKind::Other => "other",
    }
}
