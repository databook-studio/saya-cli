//! The path-matching tool: one bounded glob over the run workspace, served
//! from the contained walk. The pattern is matched against real paths only —
//! files and directories the containment layer itself resolved — so an
//! absolute pattern or a `..` prefix can never match, and symlinks are
//! neither matched nor descended into. Both bounds are the harness's typed
//! errors, surfaced as-is: a shortened match list would read as every match.

use saya_agent::ToolError;

use super::DatabaseTools;

/// The walk bound passed to the workspace: how many entries (files plus
/// directories) the contained walk may visit before refusing. Two thousand
/// entries is orders of magnitude past a hand-authored workspace — hitting it
/// means a bulk artifact dump the model should not page through blindly —
/// while staying comfortably above anything a legitimate run produces.
pub(crate) const WORKSPACE_GLOB_MAX_VISITED: usize = 2_000;

/// The match bound passed to the workspace. Past five hundred matched paths
/// the pattern is too broad to be useful — the model should narrow it (a
/// deeper prefix, a tighter extension) rather than receive a list it cannot
/// read. Refusing, not truncating, keeps "these are the matches" honest.
pub(crate) const WORKSPACE_GLOB_MAX_MATCHES: usize = 500;

impl DatabaseTools {
    /// Matches contained paths against a glob pattern. This tool never
    /// touches a connection — a workspace-only run has no selected profile —
    /// so dispatch routes it before connection resolution (see `dispatch`).
    /// The pattern is passed through unchanged: matching semantics (which
    /// wildcards span segments, that directories match too) are the
    /// harness's. With no workspace attached it denies with a typed error:
    /// no workspace, no glob.
    pub(super) async fn workspace_glob(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let pattern = arguments
            .get("pattern")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::PatternNotString)?;
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let matches = workspace
            .glob(
                pattern,
                WORKSPACE_GLOB_MAX_VISITED,
                WORKSPACE_GLOB_MAX_MATCHES,
            )
            .map_err(|error| ToolError::Workspace(error.to_string()))?;
        let paths: Vec<String> = matches.into_iter().map(|matched| matched.path).collect();
        Ok(serde_json::json!({ "matches": paths }))
    }
}
