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
/// surfaces as the tool's typed mismatch, never a partial chunk. With
/// `expected_digest` the commit goes through
/// [`Workspace::patch_range_guarded`] instead: the digest the caller
/// observed is re-verified at the commit's own fresh read, so a same-size
/// change in between is refused as the typed moved-anchor error.
pub(super) fn commit_splice(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
    range: std::ops::Range<u64>,
    expected_len: u64,
    expected_digest: Option<&str>,
    bytes: &[u8],
) -> Result<(), ToolError> {
    let commit = match expected_digest {
        Some(digest) => workspace.patch_range_guarded(rel, range, expected_len, digest, bytes),
        None => workspace.patch_range(rel, range, expected_len, bytes),
    };
    commit.map_err(|error| {
        match error {
            saya_harness::HarnessError::ContentChanged { expected, .. } => {
                // The caller observed `expected` and the commit's fresh
                // re-read hashed differently — the same-size change a length
                // check cannot see. Re-probe the current state the same way
                // the length-mismatch arm does, so the refusal names the
                // state the model re-anchors from. The read cannot fail in
                // the motivating race (the guard just hashed this file); if
                // it ever does, the measured size is the honest fallback.
                let (size, digest) = match workspace.read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES) {
                    Ok(file) => (file.size, hex_digest(&file.bytes)),
                    Err(_) => (expected_len, hex_digest(&[])),
                };
                ToolError::WorkspaceEditMoved {
                    path: rel.to_string(),
                    expected_size: Some(expected_len),
                    current_size: size,
                    expected_digest: Some(expected),
                    current_digest: digest,
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use saya_harness::workspace::Workspace;

    /// The commit-time content guard: an observed digest routes the commit
    /// through `Workspace::patch_range_guarded`, so a same-size change
    /// between the caller's read and the commit is refused as the tool's
    /// typed moved-anchor error — never a splice against stale content.
    /// Focused here (not through the tool executor) because no public API
    /// can change a file deterministically between the tool's own read and
    /// its commit inside one call.
    #[test]
    fn a_stale_observed_digest_refuses_the_commit_as_a_moved_anchor() {
        let root = std::env::temp_dir().join(format!("saya-splice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("sandbox directory must create");
        let workspace = Workspace::open(&root).expect("workspace must open");
        workspace
            .write("notes.md", b"hello world")
            .expect("seed must write");
        let observed = workspace
            .read("notes.md", 0)
            .expect("seed must read")
            .digest;
        // Same-size (11 bytes) overwrite between the observation and the
        // commit: a length guard alone cannot see this.
        workspace
            .write("notes.md", b"HELLO WORLD")
            .expect("overwrite must write");
        let error = commit_splice(&workspace, "notes.md", 0..5, 11, Some(&observed), b"WORLD")
            .expect_err("a stale observed digest must refuse the commit");
        match error {
            ToolError::WorkspaceEditMoved {
                path,
                expected_size,
                current_size,
                expected_digest,
                ..
            } => {
                assert_eq!(path, "notes.md");
                assert_eq!(expected_size, Some(11));
                assert_eq!(current_size, 11);
                assert_eq!(expected_digest, Some(observed));
            }
            other => panic!("expected the typed moved-anchor refusal, got: {other}"),
        }
        assert_eq!(
            std::fs::read(root.join("notes.md")).expect("file must survive"),
            b"HELLO WORLD",
            "a refused commit leaves the file byte-identical"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
