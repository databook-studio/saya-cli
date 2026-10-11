use super::super::ExtensionError;
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolEffect {
    ReadOnly,
    Mutating,
    Unknown,
}

/// Exact tool names and effects from trusted local configuration. MCP server
/// annotations never populate this policy or create approval.
pub struct McpPolicy {
    effects: BTreeMap<String, McpToolEffect>,
}
impl McpPolicy {
    pub fn new(effects: BTreeMap<String, McpToolEffect>) -> Result<Self, ExtensionError> {
        if effects.len() > 64 || effects.keys().any(|name| !valid_name(name)) {
            return Err(ExtensionError::Configuration);
        }
        Ok(Self { effects })
    }
    pub fn authorize(&self, name: &str) -> Result<(), ExtensionError> {
        match self.effects.get(name) {
            Some(McpToolEffect::ReadOnly) => Ok(()),
            Some(_) => Err(ExtensionError::ApprovalRequired),
            None => Err(ExtensionError::UntrustedTool),
        }
    }
}
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./".contains(&b))
}
pub(super) fn validate_tools(value: &Value) -> Result<(), ExtensionError> {
    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(ExtensionError::Protocol)?;
    if tools.len() > 64 || value.get("nextCursor").is_some() {
        return Err(ExtensionError::Limit);
    }
    let mut names = std::collections::HashSet::new();
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or(ExtensionError::Protocol)?;
        if !valid_name(name) || !names.insert(name) {
            return Err(ExtensionError::Protocol);
        }
        let schema = tool.get("inputSchema").ok_or(ExtensionError::Protocol)?;
        if schema.get("type").and_then(Value::as_str) != Some("object")
            || serde_json::to_vec(schema)
                .map_err(|_| ExtensionError::Protocol)?
                .len()
                > 16384
        {
            return Err(ExtensionError::Limit);
        }
    }
    Ok(())
}
