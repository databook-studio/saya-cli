//! The model-facing anchored edit (`replace` and `append` variants): the
//! replace variant resolves `old_text` to exactly one byte range of a
//! contained workspace file; the append variant splices `chunk` at an empty
//! range at EOF guarded by `offset`. Both commit through
//! `Workspace::patch_range` — append *is* a range replace, not a second write
//! path. Path resolution, the no-follow open, and the atomic commit are the
//! harness's — never re-derived here. This tool's own jobs are argument
//! typing, the byte bounds, the `expected_*` precondition, and typed refusals
//! that carry counts, line numbers, sizes and digests — never file content.
//!
//! Failure contract (DESIGN.md §4): every row refuses loudly and writes nothing.
//! Zero matches refuses (`NoMatch`, with the current
//! size and digest so the model re-anchors); multiple matches refuses
//! (`Ambiguous`, with bounded line numbers only, excerpts within 2 KiB —
//! never "first wins"); a moved anchor refuses (`expected_size` or
//! `expected_digest` no longer matching the current file — no write); a
//! replacement or chunk over the bound refuses whole, never truncated; an
//! offset mismatch refuses (`OffsetMismatch`, reporting the current size and
//! digest so the model resumes from `offset`); a mid-edit truncation lands
//! nothing (a truncated tool-call argument never parses or validates; a
//! crashed commit leaves the old file — temp+rename is atomic with
//! post-write re-verification); a non-UTF-8 target is refused (`NotText` —
//! reads are lossy, so byte-exact anchors cannot be trusted); concurrent writers
//! are last-writer-wins unless `expected_*` is supplied, in which
//! case the loser gets the moved-anchor refusal (no locking, no merge); an
//! empty anchor is rejected (it matches everywhere).
//!
//! What this does not do: it does not recover from an output-token cap on
//! its own — the continuation loop is unbuilt, so a stopped response resumes
//! only when the model is told to resume from the reported size and digest.
//! Chunking does not beat the context window: every continuation re-sends
//! history, so a task needing more total output than the window allows still
//! fails. `workspace_write` remains for small whole-file writes; this is not
//! its replacement.

use saya_agent::ToolError;

use super::DatabaseTools;
use super::workspace_edit_anchor::{find_matches, hex_digest, match_lines};
use super::workspace_write::WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's `old_text` and `new_text`: the same 64
/// KiB round-trip discipline as the whole-file write, so anything an edit
/// inserts can be read back whole. Over the bound is a typed whole-refusal,
/// never a truncation.
pub(crate) const WORKSPACE_EDIT_MAX_BYTES: usize = WORKSPACE_WRITE_MAX_BYTES;

/// The bound on a workspace edit's target file: the harness reads the file
/// whole to anchor the match, so targets are capped at the patch layer's
/// file cap with a typed refusal naming the size and the cap. Larger files
/// stay editable through the process lanes under their own gates.
pub(crate) const WORKSPACE_EDIT_MAX_FILE_BYTES: u64 =
    saya_harness::workspace::patch::PATCH_MAX_FILE_BYTES;

pub(super) enum WorkspaceEditRequest {
    Replace {
        path: String,
        old_text: String,
        new_text: String,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
    Append {
        path: String,
        offset: u64,
        chunk: String,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    },
}

/// Parses the two disjoint edit variants once for both schema validation and
/// execution. Keeping the shape check beside the executor prevents a caller
/// from selecting a variant by accident when fields are missing or mixed.
pub(super) fn parse_arguments(
    arguments: &serde_json::Value,
) -> Result<WorkspaceEditRequest, ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    const ALLOWED: &[&str] = &[
        "path",
        "old_text",
        "new_text",
        "offset",
        "chunk",
        "expected_size",
        "expected_digest",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    let path = object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or(ToolError::PathNotString)?
        .to_owned();
    let expected_size = match object.get("expected_size") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or(ToolError::ExpectedSizeNotUint)?),
    };
    let expected_digest = match object.get("expected_digest") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or(ToolError::ExpectedDigestNotString)?
                .to_owned(),
        ),
    };
    let has_old = object.contains_key("old_text");
    let has_new = object.contains_key("new_text");
    let has_offset = object.contains_key("offset");
    let has_chunk = object.contains_key("chunk");
    if has_offset || has_chunk {
        let offset = object
            .get("offset")
            .and_then(serde_json::Value::as_u64)
            .ok_or(ToolError::OffsetNotUint)?;
        let chunk = object
            .get("chunk")
            .and_then(serde_json::Value::as_str)
            .ok_or(ToolError::ChunkNotString)?
            .to_owned();
        if has_old || has_new {
            return Err(ToolError::UnsupportedProperty);
        }
        return Ok(WorkspaceEditRequest::Append {
            path,
            offset,
            chunk,
            expected_size,
            expected_digest,
        });
    }
    let old_text = object
        .get("old_text")
        .and_then(serde_json::Value::as_str)
        .ok_or(ToolError::OldTextNotString)?
        .to_owned();
    let new_text = object
        .get("new_text")
        .and_then(serde_json::Value::as_str)
        .ok_or(ToolError::NewTextNotString)?
        .to_owned();
    Ok(WorkspaceEditRequest::Replace {
        path,
        old_text,
        new_text,
        expected_size,
        expected_digest,
    })
}

impl DatabaseTools {
    /// Replaces the single occurrence of `old_text` in the contained file
    /// `path` with `new_text`, atomically: zero or multiple matches refuse
    /// with a typed error and write nothing — never "first wins". The
    /// optional `expected_size`/`expected_digest` precondition refuses with a
    /// typed error when the file's current state no longer matches what the
    /// model measured, so a moved anchor cannot splice against stale offsets.
    /// With `offset`+`chunk` instead, appends `chunk` at EOF under the
    /// positional precondition `offset == current size` (offset 0 on an
    /// absent path creates the file); a mismatch refuses with the current
    /// size so the model resumes from `offset` rather than guessing. Both
    /// variants commit the same way, through `Workspace::patch_range`:
    /// append is the empty range `size..size` with `offset` as the
    /// precondition, never a second write path.
    /// Like the workspace read tools this never touches a connection —
    /// dispatch routes it before connection resolution — and it records
    /// nothing into the knowledge store: a workspace edit is not evidence
    /// about a database object. With no workspace attached it denies with a
    /// typed error: no workspace, no edit.
    pub(super) async fn workspace_edit(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match parse_arguments(arguments)? {
            WorkspaceEditRequest::Replace {
                path,
                old_text,
                new_text,
                expected_size,
                expected_digest,
            } => {
                self.workspace_replace(&path, &old_text, &new_text, expected_size, expected_digest)
                    .await
            }
            WorkspaceEditRequest::Append {
                path,
                offset,
                chunk,
                expected_size,
                expected_digest,
            } => {
                self.workspace_append(&path, offset, &chunk, expected_size, expected_digest)
                    .await
            }
        }
    }

    /// The `replace` half of [`DatabaseTools::workspace_edit`]: resolve
    /// `old_text` to exactly one byte range, then commit the splice through
    /// `Workspace::patch_range`.
    async fn workspace_replace(
        &self,
        rel: &str,
        old_text: &str,
        new_text: &str,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    ) -> Result<serde_json::Value, ToolError> {
        if old_text.is_empty() {
            return Err(ToolError::WorkspaceEditEmptyAnchor {
                path: rel.to_string(),
            });
        }
        if old_text.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: old_text.len(),
            });
        }
        if new_text.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: new_text.len(),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let target_size = workspace_size(workspace, rel)?;
        if target_size > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEdit(
                saya_harness::HarnessError::BoundsExceeded {
                    path: rel.to_string(),
                    found: target_size,
                    max: WORKSPACE_EDIT_MAX_FILE_BYTES,
                }
                .to_string(),
            ));
        }
        let file = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        let current = match std::str::from_utf8(&file.bytes) {
            Ok(text) => text,
            Err(_) => {
                return Err(ToolError::WorkspaceEditNotText {
                    path: rel.to_string(),
                });
            }
        };
        let size = file.size;
        let digest = hex_digest(&file.bytes);
        if let Some(want) = expected_size
            && want != size
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size: Some(want),
                current_size: size,
                expected_digest: expected_digest.clone(),
                current_digest: digest,
            });
        }
        if let Some(want) = expected_digest.as_deref()
            && want != digest
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size,
                current_size: size,
                expected_digest: expected_digest.clone(),
                current_digest: digest,
            });
        }
        let hits = find_matches(current, old_text);
        if hits.is_empty() {
            return Err(ToolError::WorkspaceEditNoMatch {
                path: rel.to_string(),
                matches: 0,
                size,
                digest,
            });
        }
        if hits.len() > 1 {
            let lines = match_lines(current, &hits);
            return Err(ToolError::WorkspaceEditAmbiguous {
                path: rel.to_string(),
                matches: hits.len(),
                lines,
                size,
                digest,
            });
        }
        let start = hits[0] as u64;
        let end = start + old_text.len() as u64;
        commit_splice(workspace, rel, start..end, size, new_text.as_bytes())?;
        let after = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        Ok(serde_json::json!({
            "path": rel,
            "bytes_replaced": old_text.len(),
            "bytes_written": new_text.len(),
            "size": after.size,
            "digest": hex_digest(&after.bytes),
        }))
    }

    /// The `append` half of [`DatabaseTools::workspace_edit`]: an empty
    /// range at EOF with `offset` as the positional precondition, committed
    /// through the same [`Workspace::patch_range`] the replace half uses.
    /// Offset 0 on an absent path creates the file; any mismatch refuses,
    /// writes nothing, and reports the current size and digest. A supplied
    /// `expected_size` is a second precondition checked like the replace
    /// half's: a file that moved under the model refuses before the offset
    /// check, so a stated guard never silently passes.
    async fn workspace_append(
        &self,
        rel: &str,
        offset: u64,
        chunk: &str,
        expected_size: Option<u64>,
        expected_digest: Option<String>,
    ) -> Result<serde_json::Value, ToolError> {
        if chunk.len() > WORKSPACE_EDIT_MAX_BYTES {
            return Err(ToolError::WorkspaceEditTooLarge {
                path: rel.to_string(),
                limit: WORKSPACE_EDIT_MAX_BYTES,
                found: chunk.len(),
            });
        }
        let Some(workspace) = &self.workspace else {
            return Err(ToolError::WorkspaceUnavailable);
        };
        let target_size = match workspace_probe(workspace, rel)? {
            None => {
                // Absent path: only offset 0 creates. Anything else is a
                // mismatch against the (empty, absent) current state. A
                // supplied `expected_size` guards the same way: anything but
                // the absent size 0 means the file moved under the model.
                if let Some(want) = expected_size
                    && want != 0
                {
                    return Err(ToolError::WorkspaceEditMoved {
                        path: rel.to_string(),
                        expected_size: Some(want),
                        current_size: 0,
                        expected_digest: expected_digest.clone(),
                        current_digest: hex_digest(&[]),
                    });
                }
                if offset != 0 {
                    return Err(ToolError::WorkspaceAppendOffset {
                        path: rel.to_string(),
                        expected_offset: offset,
                        current_size: 0,
                        current_digest: hex_digest(&[]),
                    });
                }
                write_new(workspace, rel, chunk.as_bytes())?;
                let after = workspace
                    .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
                    .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
                return Ok(serde_json::json!({
                    "path": rel,
                    "size": after.size,
                    "digest": hex_digest(&after.bytes),
                }));
            }
            Some(size) => size,
        };
        if target_size > WORKSPACE_EDIT_MAX_FILE_BYTES {
            return Err(ToolError::WorkspaceEdit(
                saya_harness::HarnessError::BoundsExceeded {
                    path: rel.to_string(),
                    found: target_size,
                    max: WORKSPACE_EDIT_MAX_FILE_BYTES,
                }
                .to_string(),
            ));
        }
        let file = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        if std::str::from_utf8(&file.bytes).is_err() {
            return Err(ToolError::WorkspaceEditNotText {
                path: rel.to_string(),
            });
        }
        let size = file.size;
        let digest = hex_digest(&file.bytes);
        if let Some(want) = expected_size
            && want != size
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size: Some(want),
                current_size: size,
                expected_digest: expected_digest.clone(),
                current_digest: digest,
            });
        }
        if let Some(want) = expected_digest.as_deref()
            && want != digest
        {
            return Err(ToolError::WorkspaceEditMoved {
                path: rel.to_string(),
                expected_size,
                current_size: size,
                expected_digest,
                current_digest: digest,
            });
        }
        if offset != size {
            return Err(ToolError::WorkspaceAppendOffset {
                path: rel.to_string(),
                expected_offset: offset,
                current_size: size,
                current_digest: digest,
            });
        }
        commit_splice(workspace, rel, size..size, size, chunk.as_bytes())?;
        let after = workspace
            .read(rel, WORKSPACE_EDIT_MAX_FILE_BYTES)
            .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))?;
        Ok(serde_json::json!({
            "path": rel,
            "size": after.size,
            "digest": hex_digest(&after.bytes),
        }))
    }
}

/// The one commit seam both variants share: the harness's anchored
/// temp+rename commit behind [`Workspace::patch_range`], never a second
/// write path. A positional race that trips the harness's own size check
/// surfaces as the tool's typed mismatch, never a partial chunk.
fn commit_splice(
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
fn workspace_size(
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
fn workspace_probe(
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
fn write_new(
    workspace: &saya_harness::workspace::Workspace,
    rel: &str,
    bytes: &[u8],
) -> Result<(), ToolError> {
    workspace
        .write(rel, bytes)
        .map_err(|error| ToolError::WorkspaceEdit(error.to_string()))
}
