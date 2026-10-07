//! `config show --explain`: for every user-facing setting, which layer supplied it.

use saya_config::{SettingSource, user_facing_keys};

use super::explain_labels::display_key;
use super::runtime::RuntimeConfig;

/// One setting's explanation: the dotted key, the redacted value exactly as
/// plain `config show` prints it, and where the setting came from. `file`
/// appears only for file layers; `reference` only as a redacted label.
#[derive(Debug, serde::Serialize)]
pub(crate) struct SettingExplanation {
    pub(crate) key: String,
    pub(crate) value: serde_json::Value,
    pub(crate) source: String,
    pub(crate) layer: SettingSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reference: Option<String>,
}

/// One project-layer override the trust boundary ignored, with the reason.
#[derive(Debug, serde::Serialize)]
pub(crate) struct IgnoredOverride {
    pub(crate) key: String,
    pub(crate) reason: String,
}

/// The `--explain` payload: settings in the user-facing table's order, then
/// the ignored project overrides.
#[derive(Debug, serde::Serialize)]
pub(crate) struct ConfigExplanation {
    pub(crate) settings: Vec<SettingExplanation>,
    pub(crate) ignored_project_overrides: Vec<IgnoredOverride>,
}

/// Explains a composed runtime: the winning layer of every user-facing
/// setting, plus what the project layer tried to set and could not. A
/// diagnostics field with no provenance entry reports the default layer
/// (unreachable after `resolve` fills the defaults; hand-built runtimes
/// only).
pub(crate) fn explain(runtime: &RuntimeConfig) -> ConfigExplanation {
    let diagnostics = serde_json::to_value(runtime.resolved.redacted_diagnostics())
        .expect("redacted diagnostics always serialize");
    let fields = diagnostics.as_object().expect("diagnostics are an object");
    let settings = user_facing_keys()
        .iter()
        .filter_map(|(field, key)| {
            let value = fields.get(*field)?;
            let (layer, reference) = match runtime.resolved.provenance.get(key) {
                Some(origin) => (origin.source.clone(), origin.reference.clone()),
                None => (SettingSource::Default, None),
            };
            let file = file_for(&layer, runtime);
            Some(SettingExplanation {
                key: display_key(field, key),
                value: value.clone(),
                source: source_text(&layer),
                layer,
                file,
                reference,
            })
        })
        .collect();
    let ignored_project_overrides = runtime
        .resolved
        .ignored_project_overrides
        .iter()
        .map(|key| IgnoredOverride {
            key: key.clone(),
            reason: format!(
                "the project file cannot set {key}: it is security-critical; \
                 set it in your user config file or pass --trust-project-config"
            ),
        })
        .collect();
    ConfigExplanation {
        settings,
        ignored_project_overrides,
    }
}

/// The human layer name; env sources name the variable, `Cli` the flag.
fn source_text(layer: &SettingSource) -> String {
    match layer {
        SettingSource::Default => "built-in default".to_owned(),
        SettingSource::UserFile => "user config file".to_owned(),
        SettingSource::ProjectFile => "project config file".to_owned(),
        SettingSource::EnvFile { var } => format!("env file ({var})"),
        SettingSource::Env { var } => format!("environment ({var})"),
        SettingSource::Cli { flag } => format!("command line ({flag})"),
    }
}

/// The file a file layer would be read from, shown only when that layer
/// supplied the value. The project path follows the loader's selection:
/// `--config` when passed, else the discovered `.saya/config.toml` —
/// `config_path` is already that file whenever a project-layer value won.
fn file_for(layer: &SettingSource, runtime: &RuntimeConfig) -> Option<String> {
    match layer {
        SettingSource::UserFile => Some(
            super::sources::user_config_dir()
                .join("config.toml")
                .display()
                .to_string(),
        ),
        SettingSource::ProjectFile => Some(match runtime.config_path.as_ref() {
            Some(path) => path.display().to_string(),
            None => std::env::current_dir()
                .ok()?
                .join(".saya")
                .join("config.toml")
                .display()
                .to_string(),
        }),
        _ => None,
    }
}

/// The text rendering: one aligned line per setting, then the ignored
/// overrides. Strings print raw; everything else is compact JSON.
pub(crate) fn text(explanation: &ConfigExplanation) -> String {
    let mut lines: Vec<String> = explanation
        .settings
        .iter()
        .map(|setting| {
            let value = match &setting.value {
                serde_json::Value::String(raw) => raw.clone(),
                other => other.to_string(),
            };
            let mut line = format!("{:<38} {}  ← {}", setting.key, value, setting.source);
            if let Some(file) = &setting.file {
                line.push_str(&format!(" ({file})"));
            }
            line
        })
        .collect();
    if !explanation.ignored_project_overrides.is_empty() {
        lines.extend([String::new(), "Ignored from the project file:".to_owned()]);
        for ignored in &explanation.ignored_project_overrides {
            lines.push(format!("  {}", ignored.reason));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "explain_tests.rs"]
mod tests;
