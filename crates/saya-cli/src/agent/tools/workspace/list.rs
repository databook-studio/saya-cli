//! The directory-listing tool: one bounded listing from the run workspace.
//! Path resolution is delegated entirely to `saya_harness::workspace::
//! Workspace::list` — containment, symlink refusal, and the entry bound are
//! the harness's, never re-derived here. A directory past the entry bound is
//! served short with the rest counted, never refused: a quietly shortened
//! listing would read as the whole directory, but a failed call costs the
//! model a turn and a guess at how to narrow.

use saya_agent::{ToolError, ToolExecutionContext, shape_tool_result};
use std::sync::Arc;

use super::super::DatabaseTools;

/// The entry bound passed to the workspace. A directory holding more than
/// this many entries is served short — the first five hundred by name, with
/// `truncated` set and the rest counted in `omitted` — because five hundred
/// named entries already exceeds what a model can reason about in one turn.
/// The count is what tells the model to narrow its question (list a deeper
/// path) instead of paging through a flood.
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
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        let rel = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        super::search_lifetime::validate_context(&context, "workspace list")?;
        let workspace = Arc::clone(workspace);
        let guard = self.workspace_tasks.begin()?;
        let cancellation = context.cancellation.clone();
        let mut stop = super::search_lifetime::CancellationOnDrop::new(cancellation.clone());
        let path = rel.to_owned();
        let worker = tokio::task::spawn_blocking(move || {
            if cancellation.is_cancelled() {
                return (
                    Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                        "workspace list was cancelled".into(),
                    )))),
                    guard,
                );
            }
            let entries = workspace
                .list(&path, WORKSPACE_LIST_MAX_ENTRIES)
                .map_err(|error| ToolError::Workspace(error.to_string()));
            if cancellation.is_cancelled() {
                return (
                    Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                        "workspace list was cancelled".into(),
                    )))),
                    guard,
                );
            }
            (entries, guard)
        });
        let (entries, worker_guard) = worker.await.map_err(|_| {
            ToolError::Runner("workspace list worker outcome is unavailable".into())
        })?;
        stop.disarm();
        let listed = entries?;
        let mut entries = listed.entries;
        let mut omitted = listed.omitted;
        let mut truncated = listed.truncated;
        // The caller's cap bounds the whole result too. Entries the cap cannot
        // hold join the omitted count rather than failing the call, so the
        // listing stays truthful about what it left out; the entry bound caps
        // this loop at `WORKSPACE_LIST_MAX_ENTRIES` iterations.
        let result = loop {
            let result = listing(rel, &entries, truncated, omitted, listed.omitted_exact);
            if entries.is_empty() || !shape_tool_result(&result, context.result_cap).truncated {
                break result;
            }
            entries.pop();
            omitted += 1;
            truncated = true;
        };
        if context.cancellation.is_cancelled() {
            return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                "workspace list was cancelled".into(),
            ))));
        }
        drop(worker_guard);
        Ok(result)
    }
}

/// The listing as the model sees it: the served entries, and — when the
/// directory held more — that it was served short and how many entries were
/// left out. `omitted_exact` false reads as "at least this many". `kind` is a
/// machine-readable label of the harness's `EntryKind`; `size` is a file's
/// byte length (0 for directories and links).
fn listing(
    path: &str,
    entries: &[saya_harness::workspace::ListEntry],
    truncated: bool,
    omitted: u64,
    omitted_exact: bool,
) -> serde_json::Value {
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
    let mut result = serde_json::json!({
        "path": path,
        "entries": listed,
        "truncated": truncated,
        "omitted": omitted,
        "omitted_exact": omitted_exact,
    });
    // Both flags read at their default when the listing was served whole:
    // `truncated` false and `omitted_exact` true are what the description
    // states, so only the exceptional value is carried.
    crate::agent::tools::omit_default_flag(&mut result, "truncated", false);
    crate::agent::tools::omit_default_flag(&mut result, "omitted_exact", true);
    result
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
