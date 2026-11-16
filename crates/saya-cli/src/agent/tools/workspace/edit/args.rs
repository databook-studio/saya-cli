use saya_agent::ToolError;

use super::WorkspaceEditRequest;

pub(super) const MAX_MULTI_EDITS: usize = 32;
pub(super) const MAX_MULTI_EDIT_TEXT_BYTES: usize = 64 * 1024;
pub(super) const MAX_MULTI_EDIT_ARGUMENT_BYTES: usize = 128 * 1024;

pub(super) fn parse_arguments(
    arguments: &serde_json::Value,
) -> Result<WorkspaceEditRequest, ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    const ALLOWED: &[&str] = &[
        "path",
        "old_text",
        "new_text",
        "oldString",
        "old_string",
        "newString",
        "new_string",
        "offset",
        "chunk",
        "expected_size",
        "expected_digest",
        "edits",
        "replace_all",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    if object.contains_key("edits")
        && (object
            .get("path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.len() > 512)
            || object
                .get("expected_digest")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|digest| digest.len() > 64))
    {
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
    // `replace_all` names the replace form only, and it conflicts only when
    // it asks for something: `false` is the default and says nothing, so it
    // is harmless beside another shape.
    let replace_all = match object.get("replace_all") {
        None | Some(serde_json::Value::Null) => false,
        Some(value) => value.as_bool().ok_or(ToolError::ReplaceAllNotBool)?,
    };
    if replace_all
        && ["edits", "offset", "chunk"]
            .iter()
            .any(|key| object.contains_key(*key))
    {
        return Err(ToolError::ReplaceAllConflict);
    }
    if let Some(edits) = object.get("edits") {
        if object
            .keys()
            .any(|key| matches!(key.as_str(), "old_text" | "new_text" | "offset" | "chunk"))
        {
            return Err(ToolError::EditShapeConflict);
        }
        let edits = edits.as_array().ok_or(ToolError::UnsupportedProperty)?;
        if edits.is_empty() || edits.len() > MAX_MULTI_EDITS {
            return Err(ToolError::UnsupportedProperty);
        }
        let mut total_text = 0usize;
        let mut parsed = Vec::with_capacity(edits.len());
        for edit in edits {
            let fields = edit.as_object().ok_or(ToolError::UnsupportedProperty)?;
            if fields.len() != 2
                || fields
                    .keys()
                    .any(|key| !matches!(key.as_str(), "old_text" | "new_text"))
            {
                return Err(ToolError::UnsupportedProperty);
            }
            let old_text = fields
                .get("old_text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .ok_or(ToolError::UnsupportedProperty)?;
            let new_text = fields
                .get("new_text")
                .and_then(serde_json::Value::as_str)
                .ok_or(ToolError::UnsupportedProperty)?;
            total_text = total_text
                .checked_add(old_text.len())
                .and_then(|size| size.checked_add(new_text.len()))
                .filter(|size| *size <= MAX_MULTI_EDIT_TEXT_BYTES)
                .ok_or(ToolError::UnsupportedProperty)?;
            parsed.push((old_text.to_owned(), new_text.to_owned()));
        }
        if serde_json::to_vec(arguments)
            .map_err(|_| ToolError::UnsupportedProperty)?
            .len()
            > MAX_MULTI_EDIT_ARGUMENT_BYTES
        {
            return Err(ToolError::UnsupportedProperty);
        }
        return Ok(WorkspaceEditRequest::Multi {
            path,
            edits: parsed,
            expected_size,
            expected_digest,
        });
    }
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
            return Err(ToolError::EditShapeConflict);
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
    // A missing digest is not an argument error: the file has to be read
    // before the refusal can name the state the model retries from, so the
    // precondition is raised in the execute half, not here.
    Ok(WorkspaceEditRequest::Replace {
        path,
        old_text,
        new_text,
        replace_all,
        expected_size,
        expected_digest,
    })
}
