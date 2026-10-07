//! The chart save's publish step: the rendered HTML lands in the run
//! workspace through the same guarded primitives the `workspace_write` tool
//! uses — `create_new` for the create form, `write_guarded` for the
//! observed-digest replace — with the harness's refusals mapped to the same
//! texts the file tool surfaces. The chart's own size bound is the
//! whole-file I/O bound, not the model-write bound: chart HTML is rendered
//! from query results, not model-supplied text. Publication happens before
//! the private temp copy and the browser open, so a refused save opens
//! nothing; no redaction-marker guard applies (the content carries no
//! model-supplied text) and no checkpoint capture (D5: a chart is a
//! database-derived artifact, not a direct-file checkpoint).

use saya_agent::ToolError;
use saya_harness::{HarnessError, workspace::Workspace};

use super::workspace_write::{
    FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST, MISSING_FILE_NOT_A_REPLACE,
};

/// Publishes the rendered chart HTML into the run workspace under the same
/// containment as `workspace_write`: create-if-absent by default, and a
/// replacement only with the `expected_digest` the model observed from a
/// fresh read — a stale or same-size-changed file refuses, so the model is
/// never last-writer-wins. Errors map exactly like the file tool's.
pub(in crate::agent::tools) fn publish_chart_html(
    workspace: &Workspace,
    path: &str,
    html: &[u8],
    expected_digest: Option<&str>,
) -> Result<(), ToolError> {
    if html.len() > saya_harness::workspace::MAX_IO_BYTES {
        return Err(ToolError::WorkspaceWrite(format!(
            "content is over the {}-byte workspace write bound",
            saya_harness::workspace::MAX_IO_BYTES
        )));
    }
    match expected_digest {
        // Create-if-absent: an existing entry refuses whole and is left
        // byte-identical; the model must read it first to replace it.
        None => workspace
            .create_new(path, html)
            .map_err(|error| match error {
                HarnessError::AlreadyExists { .. } => {
                    ToolError::WorkspaceWrite(FILE_EXISTS_REQUIRES_DIGEST.to_owned())
                }
                other => ToolError::WorkspaceWrite(other.to_string()),
            }),
        // Guarded replace: the file must exist and its fresh re-read must
        // still hash to the digest the model observed. A missing file is
        // not a create, and a changed file is never clobbered.
        Some(digest) => workspace
            .write_guarded(path, digest, html)
            .map_err(|error| {
                if error.is_not_found() {
                    ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned())
                } else {
                    match error {
                        HarnessError::ContentChanged { .. } => {
                            ToolError::WorkspaceWrite(FILE_CHANGED_SINCE_OBSERVED.to_owned())
                        }
                        other => ToolError::WorkspaceWrite(other.to_string()),
                    }
                }
            }),
    }
}

/// The chart save's `expected_digest` argument typing: a present non-null
/// digest must be a string, and a digest without a string `save_to` refuses
/// at validation — there is no destination to guard without one, so the
/// digest is a caller error, never an ignorable argument. An explicit null
/// counts as absent (the create form), as in `workspace_write`'s parser.
pub(in crate::agent::tools) fn validate_digest_arguments(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ToolError> {
    match object.get("expected_digest") {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(digest) => {
            if digest.as_str().is_none() {
                return Err(ToolError::ExpectedDigestNotString);
            }
            if !object
                .get("save_to")
                .is_some_and(serde_json::Value::is_string)
            {
                return Err(ToolError::WorkspaceWrite(
                    "expected_digest is only valid together with save_to".into(),
                ));
            }
            Ok(())
        }
    }
}
