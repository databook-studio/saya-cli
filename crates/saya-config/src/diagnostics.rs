use serde::Serialize;

use crate::{ConfigFile, ResolvedConfig};

/// A display-safe view of configuration; references are retained, values are not.
#[derive(Debug, Clone, Serialize)]
pub struct RedactedDiagnostics {
    pub default_profile: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_reference: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub max_rows: Option<usize>,
}

/// A display-safe view of effective runtime settings with no resolved secrets.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDiagnostics {
    pub profile_name: Option<String>,
    pub profile_dialect: Option<String>,
    pub provider: String,
    pub model: String,
    pub allow_data_sharing: bool,
    pub max_rows: usize,
    pub read_only: bool,
}

impl RedactedDiagnostics {
    pub(crate) fn from_file(file: &ConfigFile) -> Self {
        Self {
            default_profile: file.default_profile.clone(),
            provider: file.ai.provider.map(|value| value.as_str().into()),
            model: file.ai.model.clone(),
            api_key_reference: file.ai.api_key.as_ref().map(|value| value.redacted_label()),
            allow_data_sharing: file.ai.allow_data_sharing,
            max_rows: file.run.max_rows,
        }
    }
}

impl ResolvedConfig {
    pub fn redacted_diagnostics(&self) -> ResolvedDiagnostics {
        ResolvedDiagnostics {
            profile_name: self.profile_name.clone(),
            profile_dialect: self
                .profile
                .as_ref()
                .map(|value| value.dialect().as_str().into()),
            provider: self.ai.provider.as_str().into(),
            model: self.ai.model.clone(),
            allow_data_sharing: self.ai.allow_data_sharing,
            max_rows: self.max_rows,
            read_only: self.read_only,
        }
    }
}
