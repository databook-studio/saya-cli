use saya_agent::ToolError;

use super::workspace_edit::WorkspaceEditRequest;

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
