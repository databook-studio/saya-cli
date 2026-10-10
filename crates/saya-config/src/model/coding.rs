use serde::Deserialize;

/// Raw trusted `[coding]` settings. Every field stays optional until layers
/// merge, so a higher-precedence layer can override one setting at a time.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CodingFile {
    pub enabled: Option<bool>,
    pub max_children: Option<usize>,
    pub max_concurrent_model_requests: Option<usize>,
    pub max_concurrent_commands: Option<usize>,
    pub max_logical_model_requests: Option<usize>,
    pub max_physical_transport_attempts: Option<usize>,
    pub max_tool_calls: Option<usize>,
    pub max_known_tokens: Option<u64>,
    pub max_elapsed_seconds: Option<u64>,
}
