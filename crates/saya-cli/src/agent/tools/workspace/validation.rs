use saya_agent::ToolError;

use super::grep_types::{self, MAX_CONTEXT};
use super::names::canonical_name;
use super::search_lifetime::refusal;

pub(in crate::agent::tools) fn is_tool_name(name: &str) -> bool {
    matches!(
        canonical_name(name),
        "read" | "list" | "write" | "edit" | "notebook_edit" | "delete" | "glob" | "grep"
    )
}

pub(in crate::agent::tools) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    let name = canonical_name(name);
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if name == "edit" {
        super::edit::validate_arguments(arguments)?;
    } else if name == "notebook_edit" {
        super::notebook::validate_arguments(arguments)?;
    } else {
        let allowed = match name {
            "read" => &["path", "start_line", "line_count"][..],
            "list" => &["path"][..],
            "write" => &["path", "content", "expected_digest"][..],
            "delete" => &["path", "expected_digest"][..],
            "glob" => &["pattern", "path", "type"][..],
            "grep" => &[
                "pattern",
                "path",
                "include",
                "regex",
                "case_insensitive",
                "output_mode",
                "context",
                "type",
                "multiline",
                "head_limit",
            ][..],
            _ => return Err(ToolError::UnsupportedTool),
        };
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    if matches!(name, "read" | "delete")
        && !object.get("path").is_some_and(serde_json::Value::is_string)
    {
        return Err(ToolError::PathNotString);
    }
    if name == "read" {
        if object
            .get("path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.len() > 512)
        {
            return Err(ToolError::UnsupportedProperty);
        }
        for (key, default, max) in [
            ("start_line", 1u64, u64::MAX),
            ("line_count", 1, super::read::WORKSPACE_READ_MAX_LINES),
        ] {
            if let Some(value) = object.get(key) {
                let number = value.as_u64().ok_or(ToolError::UnsupportedProperty)?;
                if number < default || number > max {
                    return Err(ToolError::UnsupportedProperty);
                }
            }
        }
        if let (Some(start), Some(count)) = (
            object.get("start_line").and_then(serde_json::Value::as_u64),
            object.get("line_count").and_then(serde_json::Value::as_u64),
        ) && start.checked_add(count - 1).is_none()
        {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    if name == "delete"
        && !object
            .get("expected_digest")
            .is_some_and(serde_json::Value::is_string)
    {
        return Err(ToolError::ExpectedDigestNotString);
    }
    if name == "write" {
        if !object.get("path").is_some_and(serde_json::Value::is_string) {
            return Err(ToolError::PathNotString);
        }
        if !object
            .get("content")
            .is_some_and(serde_json::Value::is_string)
        {
            return Err(ToolError::ContentNotString);
        }
        if object
            .get("expected_digest")
            .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(ToolError::ExpectedDigestNotString);
        }
    }
    if name == "list" && object.get("path").is_some_and(|value| !value.is_string()) {
        return Err(ToolError::PathNotString);
    }
    if matches!(name, "glob" | "grep")
        && !object
            .get("pattern")
            .is_some_and(serde_json::Value::is_string)
    {
        return Err(ToolError::PatternNotString);
    }
    if name == "grep"
        && object
            .get("case_insensitive")
            .is_some_and(|value| value.as_bool().is_none())
    {
        return Err(ToolError::CaseInsensitiveNotBool);
    }
    if name == "grep" {
        validate_grep_options(object)?;
    }
    if name == "glob"
        && object
            .get("type")
            .is_some_and(|value| !matches!(value.as_str(), Some("file" | "dir")))
    {
        return Err(ToolError::UnsupportedProperty);
    }
    if matches!(name, "glob" | "grep") {
        if object["pattern"].as_str().is_some_and(|pattern| {
            pattern.is_empty() || pattern.len() > 1024 || pattern.contains('\0')
        }) {
            return Err(ToolError::UnsupportedProperty);
        }
        if object
            .get("path")
            .is_some_and(|value| value.as_str().is_none_or(|path| path.len() > 512))
        {
            return Err(ToolError::PathNotString);
        }
        if object
            .get("include")
            .is_some_and(|value| value.as_str().is_none_or(|pattern| pattern.len() > 1024))
        {
            return Err(ToolError::UnsupportedProperty);
        }
        if object.get("regex").is_some_and(|value| !value.is_boolean()) {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    Ok(())
}

/// The grep-only arguments. Each is bounded by the same number the harness
/// itself enforces, and each refusal names what the model can fix: an
/// unknown `type` is answered with the names that exist, and `context` with
/// `multiline` is refused outright rather than silently ignoring one.
fn validate_grep_options(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ToolError> {
    if object.get("output_mode").is_some_and(|value| {
        !matches!(
            value.as_str(),
            Some("content" | "files_with_matches" | "count")
        )
    }) {
        return Err(ToolError::UnsupportedProperty);
    }
    let context = object.get("context");
    if context.is_some_and(|value| {
        value
            .as_u64()
            .is_none_or(|lines| lines > MAX_CONTEXT as u64)
    }) {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("multiline")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(ToolError::UnsupportedProperty);
    }
    let ceiling = grep_types::head_limit_ceiling() as u64;
    if object.get("head_limit").is_some_and(|value| {
        value
            .as_u64()
            .is_none_or(|limit| limit == 0 || limit > ceiling)
    }) {
        return Err(ToolError::UnsupportedProperty);
    }
    if let Some(value) = object.get("type") {
        let name = value.as_str().ok_or(ToolError::UnsupportedProperty)?;
        if grep_types::lookup(name).is_none() {
            return Err(ToolError::GrepTypeUnsupported {
                requested: name.chars().take(32).collect(),
                supported: grep_types::NAMES,
            });
        }
    }
    if object.get("multiline").and_then(serde_json::Value::as_bool) == Some(true)
        && context
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|lines| lines > 0)
    {
        return Err(refusal(
            "grep: `context` cannot be combined with `multiline`",
        ));
    }
    Ok(())
}
