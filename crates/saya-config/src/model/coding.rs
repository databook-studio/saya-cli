use std::collections::BTreeMap;
use std::fmt;

use saya_types::CheckDiagnosticFormat;
use serde::Deserialize;

/// Raw trusted `[coding]` settings. Every field stays optional until layers
/// merge, so a higher-precedence layer can override one setting at a time.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CodingFile {
    pub enabled: Option<bool>,
    #[serde(default)]
    pub extensions: Option<BTreeMap<String, crate::coding::ExtensionProfile>>,
    pub max_children: Option<usize>,
    pub max_concurrent_model_requests: Option<usize>,
    pub max_concurrent_commands: Option<usize>,
    pub max_logical_model_requests: Option<usize>,
    pub max_physical_transport_attempts: Option<usize>,
    pub max_tool_calls: Option<usize>,
    pub max_known_tokens: Option<u64>,
    pub max_elapsed_seconds: Option<u64>,
    #[serde(default)]
    pub check_profiles: Option<BTreeMap<String, CheckProfileFile>>,
}

/// Raw fixed command data for one named coding check profile.
#[derive(Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckProfileFile {
    pub(crate) command_id: String,
    pub(crate) program: String,
    #[serde(default)]
    pub(crate) argv: Vec<String>,
    pub(crate) timeout_seconds: u64,
    pub(crate) toolchain: Option<String>,
    pub(crate) environment_profile: String,
    #[serde(default)]
    pub(crate) diagnostic_format: Option<CheckDiagnosticFormat>,
}

impl fmt::Debug for CheckProfileFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("CheckProfileFile");
        debug
            .field("command_id", &self.command_id)
            .field("program", &self.program)
            .field("argv_count", &self.argv.len())
            .field("timeout_seconds", &self.timeout_seconds)
            .field("toolchain", &self.toolchain)
            .field("environment_profile", &self.environment_profile);
        if let Some(format) = self.diagnostic_format {
            debug.field("diagnostic_format", &format);
        }
        debug.finish()
    }
}
