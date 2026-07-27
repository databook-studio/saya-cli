use serde::Serialize;

use crate::{ColorChoice, ConfigFile, OutputFormat, ResolvedConfig};

/// A display-safe view of configuration; references are retained, values are not.
#[derive(Debug, Clone, Serialize)]
pub struct RedactedDiagnostics {
    pub default_profile: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub read_only: Option<bool>,
    pub max_rows: Option<usize>,
    pub max_iterations: Option<usize>,
    pub query_timeout_seconds: Option<u64>,
    pub output_format: Option<OutputFormat>,
    pub output_color: Option<ColorChoice>,
}

/// A display-safe view of effective runtime settings with no resolved secrets.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDiagnostics {
    pub profile_name: Option<String>,
    pub profile_dialect: Option<String>,
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key_reference: Option<String>,
    pub allow_data_sharing: bool,
    pub max_rows: usize,
    pub read_only: bool,
    pub max_iterations: usize,
    pub query_timeout_seconds: u64,
    pub output_format: OutputFormat,
    pub output_color: ColorChoice,
}

impl RedactedDiagnostics {
    pub(crate) fn from_file(file: &ConfigFile) -> Self {
        Self {
            default_profile: file.default_profile.clone(),
            provider: file.ai.provider.map(|value| value.as_str().into()),
            model: file.ai.model.clone(),
            base_url: file.ai.base_url.as_deref().map(redact_endpoint),
            api_key_reference: file.ai.api_key.as_ref().map(|value| value.redacted_label()),
            allow_data_sharing: file.ai.allow_data_sharing,
            read_only: file.run.read_only,
            max_rows: file.run.max_rows,
            max_iterations: file.run.max_iterations,
            query_timeout_seconds: file.run.query_timeout_seconds,
            output_format: file.output.format,
            output_color: file.output.color,
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
            base_url: self.ai.base_url.as_deref().map(redact_endpoint),
            api_key_reference: self.ai.api_key.as_ref().map(|value| value.redacted_label()),
            allow_data_sharing: self.ai.allow_data_sharing,
            max_rows: self.max_rows,
            read_only: self.read_only,
            max_iterations: self.max_iterations,
            query_timeout_seconds: self.query_timeout_seconds,
            output_format: self.output_format,
            output_color: self.output_color,
        }
    }
}

fn redact_endpoint(value: &str) -> String {
    let mut output = redact_userinfo(value);
    if let Some(index) = output.find('?') {
        output.truncate(index + 1);
        output.push_str("[redacted]");
    }
    output
}

fn redact_userinfo(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(offset) = value[cursor..].find("://") {
        let scheme = cursor + offset;
        let auth_start = scheme + 3;
        let rest = &value[auth_start..];
        let Some(at_offset) = rest.find('@') else {
            break;
        };
        let boundary = rest
            .find(|character: char| "/?# \t\r\n".contains(character))
            .unwrap_or(rest.len());
        if at_offset >= boundary {
            cursor = auth_start;
            continue;
        }
        let at = auth_start + at_offset;
        output.push_str(&value[cursor..auth_start]);
        cursor = at + 1;
    }
    output.push_str(&value[cursor..]);
    output
}
