use saya_agent::ToolError;

pub(in crate::agent::tools) fn is_tool_name(name: &str) -> bool {
    matches!(
        name,
        "workspace_read"
            | "workspace_list"
            | "workspace_write"
            | "workspace_edit"
            | "glob"
            | "grep"
    )
}

pub(in crate::agent::tools) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if name == "workspace_edit" {
        super::edit::validate_arguments(arguments)?;
    } else {
        let allowed = match name {
            "workspace_read" | "workspace_list" => &["path"][..],
            "workspace_write" => &["path", "content", "expected_digest"][..],
            "glob" => &["pattern"][..],
            "grep" => &["pattern", "case_insensitive"][..],
            _ => return Err(ToolError::UnsupportedTool),
        };
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    if name == "workspace_read" && !object.get("path").is_some_and(serde_json::Value::is_string) {
        return Err(ToolError::PathNotString);
    }
    if name == "workspace_write" {
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
    if name == "workspace_list" && object.get("path").is_some_and(|value| !value.is_string()) {
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
    Ok(())
}
