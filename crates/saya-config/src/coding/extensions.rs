use crate::ConfigError;
use serde::Deserialize;
use std::{collections::BTreeMap, fmt};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionProtocol {
    Mcp,
    Lsp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionToolEffect {
    ReadOnly,
    Mutating,
    Unknown,
}

/// Trusted fixed server data. Neither schema nor effect metadata grants access.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionProfile {
    protocol: ExtensionProtocol,
    program: String,
    #[serde(default)]
    argv: Vec<String>,
    language: Option<String>,
    #[serde(default)]
    tools: BTreeMap<String, ExtensionToolEffect>,
}
impl ExtensionProfile {
    pub fn redacted_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"protocol":match self.protocol {ExtensionProtocol::Mcp=>"mcp",ExtensionProtocol::Lsp=>"lsp"},"program":self.program,"argv_count":self.argv.len(),"language":self.language,"tool_policy_count":self.tools.len()})
    }
    pub fn protocol(&self) -> ExtensionProtocol {
        self.protocol
    }
    pub fn program(&self) -> &str {
        &self.program
    }
    pub fn argv(&self) -> &[String] {
        &self.argv
    }
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }
    pub fn tools(&self) -> &BTreeMap<String, ExtensionToolEffect> {
        &self.tools
    }
}
impl fmt::Debug for ExtensionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionProfile")
            .field("protocol", &self.protocol)
            .field("program", &self.program)
            .field("argv_count", &self.argv.len())
            .field("tool_count", &self.tools.len())
            .finish_non_exhaustive()
    }
}
pub(super) fn resolve(
    profiles: Option<&BTreeMap<String, ExtensionProfile>>,
) -> Result<BTreeMap<String, ExtensionProfile>, ConfigError> {
    let Some(profiles) = profiles else {
        return Ok(BTreeMap::new());
    };
    if profiles.len() > 8 {
        return Err(ConfigError::InvalidCodingExtension);
    }
    for (name, profile) in profiles {
        if !safe_name(name)
            || !saya_types::is_bare_name(&profile.program)
            || profile.program.len() > 128
            || profile.argv.len() > 64
            || profile
                .argv
                .iter()
                .any(|arg| arg.len() > 8192 || arg.contains('\0'))
            || profile.argv.iter().map(String::len).sum::<usize>() > 32768
            || profile.tools.len() > 64
            || profile.tools.keys().any(|name| !safe_name(name))
            || (profile.protocol == ExtensionProtocol::Lsp
                && profile
                    .language
                    .as_deref()
                    .is_none_or(|language| !safe_name(language)))
            || (profile.protocol == ExtensionProtocol::Mcp && profile.language.is_some())
        {
            return Err(ConfigError::InvalidCodingExtension);
        }
    }
    Ok(profiles.clone())
}
fn safe_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-./".contains(&byte))
}
