use saya_agent::ToolError;

use super::WorkspaceEditRequest;

/// The replace variant's missing-digest refusal, raised before any file
/// access: replacing an existing file requires the digest the model
/// observed from a fresh `workspace_read` (or from the previous edit's
/// result) — never the file's current digest, which the model has not seen.
pub(super) const REPLACE_REQUIRES_DIGEST: &str = "editing an existing file requires \
    expected_digest from a fresh workspace_read (or the previous edit's result); \
    the file was not changed";

/// The append variant's missing-digest refusal for an existing file,
/// raised before the file's bytes are read into any payload: appending to
/// an existing file requires the digest the model observed from a fresh
/// `workspace_read` (or from the previous edit's result) — never the
/// file's current digest, which the model has not seen.
pub(super) const APPEND_REQUIRES_DIGEST: &str = "appending to an existing file requires \
    expected_digest from a fresh workspace_read (or the previous edit's result); \
    the file was not changed";

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
    let Some(expected_digest) = expected_digest else {
        return Err(ToolError::WorkspaceEdit(REPLACE_REQUIRES_DIGEST.to_owned()));
    };
    Ok(WorkspaceEditRequest::Replace {
        path,
        old_text,
        new_text,
        expected_size,
        expected_digest,
    })
}
