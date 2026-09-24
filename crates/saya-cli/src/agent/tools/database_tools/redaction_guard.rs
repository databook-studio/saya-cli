//! The redaction-placeholder guard `workspace_write` and `workspace_edit`
//! share: a write is refused when the content it would land holds more
//! literal `[redacted]` markers than the file already holds on disk (0 for a
//! file that does not yet exist). Closes the loss path where the model reads
//! back its own tool result, sees the D8 scrub's `[redacted]` and mistakes
//! it for corruption, then "fixes" the file by writing the placeholder text
//! over the real value — destroying it. That protection is unaffected by
//! this guard: `redact()` in `saya-types` is unchanged. This only stops the
//! placeholder from being written back into a file.

use saya_agent::ToolError;
use saya_harness::workspace::Workspace;

/// The literal marker `saya_types::redaction::redact` writes over
/// secret-shaped material. Counted over raw bytes, not decoded text, so a
/// target that is not valid UTF-8 still gets an honest count — the guard
/// must never be the reason a binary file's containment check is skipped.
const MARKER: &[u8] = b"[redacted]";

/// The file-size ceiling the guard reads an existing target under before
/// counting: the same bound `workspace_edit` already reads its anchor target
/// under (`saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES`), so the
/// probe never serves, or silently under-counts, more of a file than the
/// harness itself is willing to hand any workspace tool.
pub(super) const MARKER_PROBE_MAX_BYTES: u64 = saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

/// Counts non-overlapping occurrences of [`MARKER`] in `bytes`.
pub(super) fn count_markers(bytes: &[u8]) -> usize {
    let mut count = 0;
    let mut cursor = 0;
    while cursor + MARKER.len() <= bytes.len() {
        if bytes[cursor..cursor + MARKER.len()] == *MARKER {
            count += 1;
            cursor += MARKER.len();
        } else {
            cursor += 1;
        }
    }
    count
}

/// The marker count already on disk at `rel`: 0 for an absent file. Any
/// other read failure (containment refusal, over the probe bound) is
/// returned to the caller rather than assumed absent — a guard that cannot
/// see the current state must not silently let a write through.
pub(super) fn existing_marker_count(
    workspace: &Workspace,
    rel: &str,
) -> Result<usize, saya_harness::HarnessError> {
    match workspace.read(rel, MARKER_PROBE_MAX_BYTES) {
        Ok(file) => Ok(count_markers(&file.bytes)),
        Err(error) if error.is_not_found() => Ok(0),
        Err(error) => Err(error),
    }
}

/// Refuses when `new_content` would hold more `[redacted]` markers than
/// `before`, the count already on disk. `map_error` builds the caller's own
/// typed refusal (`ToolError::WorkspaceWrite` or `ToolError::WorkspaceEdit`)
/// from the message, so each tool's error still names its own operation.
pub(super) fn refuse_marker_growth(
    path: &str,
    before: usize,
    new_content: &[u8],
    map_error: impl FnOnce(String) -> ToolError,
) -> Result<(), ToolError> {
    let after = count_markers(new_content);
    if after > before {
        return Err(map_error(format!(
            "refused: the new content holds {after} occurrence(s) of the literal \
             `[redacted]` marker, more than the {before} already on disk: {path}. This \
             looks like a masked secret from a tool result being copied back verbatim, \
             not a real edit — the real value on disk is unchanged. Edit around the \
             masked span instead of retyping it."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::count_markers;

    #[test]
    fn counts_non_overlapping_markers() {
        assert_eq!(count_markers(b"no markers here"), 0);
        assert_eq!(count_markers(b"a = f([redacted])"), 1);
        assert_eq!(
            count_markers(b"[redacted] and again [redacted]"),
            2,
            "two separate occurrences must both count"
        );
    }

    #[test]
    fn does_not_count_the_longer_private_key_marker() {
        // `[redacted private key]` does not contain the literal `[redacted]`
        // substring (no closing bracket right after `redacted`), so the two
        // marker families never double-count each other.
        assert_eq!(count_markers(b"[redacted private key]"), 0);
    }
}
