use std::collections::BTreeMap;

use crate::{
    AiProvider, CliOverrides, CompactionMode, ConfigError, ConfigFile, OutputFormat,
    provenance::{Recorder, SettingSource, record_endpoint_fields},
};
use saya_types::SecretRef;

#[path = "layers/cli.rs"]
mod cli;
#[path = "layers/environment.rs"]
mod environment;
#[path = "layers/merge.rs"]
mod merge_layer;
#[path = "layers/protected.rs"]
mod protected;

pub(crate) use cli::apply_cli;
pub(crate) use environment::apply_env;
pub(crate) use merge_layer::merge;
pub(crate) use protected::{revert_untrusted, snapshot_protected};

fn revert_ai_budget(
    value: &mut Option<toml::Value>,
    trusted: &Option<toml::Value>,
    name: &'static str,
    ignored: &mut Vec<String>,
) {
    if value != trusted {
        *value = trusted.clone();
        ignored.push(format!("ai.{name}"));
    }
}

fn revert_instruction<T: Clone + PartialEq>(
    value: &mut Option<T>,
    trusted: &Option<T>,
    name: &'static str,
    ignored: &mut Vec<String>,
) {
    if value != trusted {
        *value = trusted.clone();
        ignored.push(format!("instructions.{name}"));
    }
}

fn apply_string(
    target: &mut Option<String>,
    env: &BTreeMap<String, String>,
    var: &str,
    key: &str,
    recorder: &mut Recorder,
    source: &dyn Fn(&str) -> SettingSource,
) {
    if let Some(value) = env.get(var) {
        *target = Some(value.clone());
        recorder.record(key, source(var));
    }
}

/// The API key is never read from the environment: `contains_key` matches
/// the name alone, the recorded reference is built from the variable NAME,
/// and no value ever flows into the recorder.
fn apply_secret(
    target: &mut Option<saya_types::SecretRef>,
    env: &BTreeMap<String, String>,
    var: &str,
    key: &str,
    recorder: &mut Recorder,
    source: &dyn Fn(&str) -> SettingSource,
) {
    if env.contains_key(var) {
        let reference = saya_types::SecretRef::Env { env: var.into() };
        *target = Some(reference.clone());
        recorder.record_secret(key, source(var), &reference);
    }
}

fn apply_parsed<T: Copy>(
    target: &mut Option<T>,
    env: &BTreeMap<String, String>,
    var: &str,
    key: &str,
    recorder: &mut Recorder,
    source: &dyn Fn(&str) -> SettingSource,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<(), ConfigError> {
    if let Some(value) = env.get(var) {
        *target = Some(parse(value).ok_or_else(|| ConfigError::InvalidEnvironment {
            name: var.into(),
            reason: "invalid value".into(),
        })?);
        recorder.record(key, source(var));
    }
    Ok(())
}

fn parse_value<T: std::str::FromStr>(value: &str) -> Option<T> {
    value.parse().ok()
}
