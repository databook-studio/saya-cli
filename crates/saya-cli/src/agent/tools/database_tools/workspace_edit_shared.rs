use saya_agent::ToolError;

use super::workspace_edit::WORKSPACE_EDIT_MAX_FILE_BYTES;
use super::workspace_edit_anchor::hex_digest;

/// The `replace` half's resulting content, built in memory so the
/// redaction-placeholder guard can inspect what the commit *would* write
/// before any byte lands on disk: `current` with the byte range `range`
/// (always a valid `old_text` match, so always a char boundary) swapped for
/// `new_text`.
pub(super) fn splice_text(current: &str, range: std::ops::Range<usize>, new_text: &str) -> String {
    let mut resulting =
        String::with_capacity(current.len() - (range.end - range.start) + new_text.len());
    resulting.push_str(&current[..range.start]);
    resulting.push_str(new_text);
    resulting.push_str(&current[range.end..]);
    resulting
}

/// The one commit seam both variants share: the harness's anchored
/// temp+rename commit behind [`Workspace::patch_range`], never a second
/// write path. A positional race that trips the harness's own size check
/// surfaces as the tool's typed mismatch, never a partial chunk.
pub(super) fn commit_splice(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
    range: std::ops::Range<u64>,
    expected_len: u64,
    bytes: &[u8],
) -> Result<(), ToolError> {
    workspace
        .patch_range(rel, range, expected_len, bytes)
        .map_err(|error| {
            match error {
                saya_harness::HarnessError::LengthMismatch { current, .. } => {
                    // The read above measured `expected_len`; the file moved
                    // before the commit. Re-probe the current state so the
                    // refusal names the size the model resumes from.
                    let (size, digest) = match workspace.read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES) {
                        Ok(file) => (file.size, hex_digest(&file.bytes)),
                        Err(_) => (current, hex_digest(&[])),
                    };
                    ToolError::WorkspaceAppendOffset {
                        path: rel.to_string(),
                        expected_offset: expected_len,
                        current_size: size,
                        current_digest: digest,
                    }
                }
                other => ToolError::WorkspaceEdit(other.to_string()),
            }
        })
}

/// The target's pre-read size probe: the contained scan behind
/// [`Workspace::read`] names the size even when the content would be capped,
/// and this probe keeps the over-cap refusal typed without serving bytes the
/// edit never needs. A missing or refused name surfaces as the edit's own
/// containment error.
pub(super) fn workspace_size(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
) -> Result<u64, ToolError> {
    let probe = workspace
        .read(rel, 0)
        .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
    Ok(probe.size)
}

/// The append half's existence probe: `Some(size)` when the path names an
/// existing target, `None` when it is absent (offset 0 then creates). Any
/// other refusal — traversal, symlink, directory — surfaces as the edit's
/// own containment error, never as absence.
pub(super) fn workspace_probe(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
) -> Result<Option<u64>, ToolError> {
    match workspace.read(rel, 0) {
        Ok(probe) => Ok(Some(probe.size)),
        Err(error) => {
            // Absence is the harness's own structure (`NotFound` or an I/O
            // error of kind `NotFound`) — never rendered strings, whose
            // "scan"/"open" contexts wrap every I/O failure including
            // permission denials. Anything else is a real refusal, not
            // absence.
            if error.is_not_found() {
                Ok(None)
            } else {
                Err(ToolError::WorkspaceEdit(error.to_string()))
            }
        }
    }
}

/// Creates the absent path the append half names, through the same atomic
/// temp+rename commit the splice uses: the harness's contained write, never
/// a second mechanism. The parent walk still refuses escapes and links.
pub(super) fn write_new(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
    bytes: &[u8],
) -> Result<(), ToolError> {
    workspace
        .write(rel, bytes)
        .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))
}
