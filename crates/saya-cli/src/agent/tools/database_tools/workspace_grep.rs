//! The text-search tool: one bounded literal-substring search over the run
//! workspace, served from the contained walk. The needle is literal — no
//! regex, so a pattern cannot be confused with an execution — and every
//! bound is the harness's typed error, surfaced as-is. Coverage is reported,
//! not implied: `files_scanned` and `files_skipped` travel with the matches,
//! because a capped search that rendered as an empty result would read as
//! proof of absence.

use saya_agent::ToolError;

use super::DatabaseTools;

/// The walk bound passed to the workspace: how many entries (files plus
/// directories) the contained walk may visit before refusing. Matches the
/// glob walk bound — one workspace, one sense of "too big to search" — so a
/// run that fails one search's coverage budget fails the other's too.
pub(crate) const WORKSPACE_GREP_MAX_VISITED: usize = 2_000;

/// The match bound passed to the workspace. Past five hundred hit lines the
/// needle is too common to be informative — the model should search a
/// narrower needle or a deeper path rather than receive hits it cannot read.
/// Refusing, not truncating, keeps "these are the hits" honest.
pub(crate) const WORKSPACE_GREP_MAX_MATCHES: usize = 500;

/// The per-hit line bound. A single line longer than two kilobytes — a
/// minified bundle, a base64 blob — is capped at this many bytes with the
/// cap reported on the hit: enough text to identify the line and its
/// context, small enough that one enormous line cannot blow the context.
pub(crate) const WORKSPACE_GREP_MAX_LINE_BYTES: usize = 2_000;

/// The per-file search horizon: a file larger than this is skipped whole,
/// never half-searched, because hits over a prefix would read as full
/// coverage. It matches the edit target cap, so every file an edit can
/// anchor is searchable whole — the locate half of the >64 KiB workflow.
/// Only bounded hit lines reach the model, never the scanned bytes.
pub(crate) const WORKSPACE_GREP_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

impl DatabaseTools {
    /// Searches workspace files for a literal substring. This tool never
    /// touches a connection — a workspace-only run has no selected profile —
    /// so dispatch routes it before connection resolution (see `dispatch`).
    /// Each file is read under [`WORKSPACE_GREP_MAX_FILE_BYTES`], the same
    /// horizon an edit can anchor: a file that horizon would truncate is
    /// skipped whole, never half-searched, because hits over a prefix would
    /// read as full coverage. Only bounded hit lines reach the model, never
    /// the scanned bytes. `case_insensitive` defaults to false. With no
    /// workspace attached it denies with a typed error: no workspace, no
    /// search.
    pub(super) async fn workspace_grep(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let needle = arguments
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::PatternNotString)?;
        let case_insensitive = match arguments.get("case_insensitive") {
            Some(value) => value.as_bool().ok_or(ToolError::CaseInsensitiveNotBool)?,
            None => false,
        };
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let outcome = workspace
            .grep(
                needle,
                case_insensitive,
                WORKSPACE_GREP_MAX_VISITED,
                WORKSPACE_GREP_MAX_MATCHES,
                WORKSPACE_GREP_MAX_FILE_BYTES,
                WORKSPACE_GREP_MAX_LINE_BYTES,
            )
            .map_err(|error| ToolError::Workspace(error.to_string()))?;
        let hits: Vec<serde_json::Value> = outcome
            .matches
            .iter()
            .map(|hit| {
                serde_json::json!({
                    "path": hit.path,
                    "line": hit.line,
                    "text": hit.text,
                    "truncated": hit.truncated,
                })
            })
            .collect();
        // Both counts travel with the matches: `files_skipped` is the
        // difference between "searched and found nothing" and "never read
        // this file", and swallowing it would turn a capped search into
        // false evidence of absence.
        Ok(serde_json::json!({
            "matches": hits,
            "files_scanned": outcome.files_scanned,
            "files_skipped": outcome.files_skipped,
        }))
    }
}
