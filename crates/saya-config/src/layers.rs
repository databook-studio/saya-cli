use std::collections::BTreeMap;

use crate::{AiProvider, CliOverrides, ConfigError, ConfigFile, OutputFormat};

pub(crate) fn merge(base: &mut ConfigFile, layer: &ConfigFile) {
    macro_rules! apply { ($($path:ident).+) => { if layer.$($path).+.is_some() { base.$($path).+ = layer.$($path).+.clone(); } }; }
    apply!(default_profile);
    apply!(ai.provider);
    apply!(ai.model);
    apply!(ai.base_url);
    apply!(ai.allow_data_sharing);
    apply!(ai.api_key);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.query_timeout_seconds);
    apply!(output.format);
    apply!(output.color);
}

pub(crate) fn apply_env(
    file: &mut ConfigFile,
    env: &BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    apply_string(&mut file.ai.model, env, "SAYA_AI_MODEL");
    apply_string(&mut file.ai.base_url, env, "SAYA_AI_BASE_URL");
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_AI_PROVIDER",
        AiProvider::parse,
    )?;
    apply_parsed(
        &mut file.ai.allow_data_sharing,
        env,
        "SAYA_ALLOW_DATA_SHARING",
        parse_value,
    )?;
    apply_parsed(&mut file.run.read_only, env, "SAYA_READ_ONLY", parse_value)?;
    apply_parsed(&mut file.run.max_rows, env, "SAYA_MAX_ROWS", parse_value)?;
    apply_parsed(
        &mut file.run.max_iterations,
        env,
        "SAYA_MAX_ITERATIONS",
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.query_timeout_seconds,
        env,
        "SAYA_QUERY_TIMEOUT_SECONDS",
        parse_value,
    )?;
    apply_parsed(
        &mut file.output.format,
        env,
        "SAYA_OUTPUT_FORMAT",
        OutputFormat::parse,
    )?;
    Ok(())
}

pub(crate) fn apply_cli(file: &mut ConfigFile, cli: &CliOverrides) {
    if cli.provider.is_some() {
        file.ai.provider = cli.provider;
    }
    if cli.model.is_some() {
        file.ai.model = cli.model.clone();
    }
    if cli.allow_data_sharing.is_some() {
        file.ai.allow_data_sharing = cli.allow_data_sharing;
    }
    if cli.max_rows.is_some() {
        file.run.max_rows = cli.max_rows;
    }
}

fn apply_string(target: &mut Option<String>, env: &BTreeMap<String, String>, name: &str) {
    if let Some(value) = env.get(name) {
        *target = Some(value.clone());
    }
}

fn apply_parsed<T: Copy>(
    target: &mut Option<T>,
    env: &BTreeMap<String, String>,
    name: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<(), ConfigError> {
    if let Some(value) = env.get(name) {
        *target = Some(parse(value).ok_or_else(|| ConfigError::InvalidEnvironment {
            name: name.into(),
            reason: "invalid value".into(),
        })?);
    }
    Ok(())
}

fn parse_value<T: std::str::FromStr>(value: &str) -> Option<T> {
    value.parse().ok()
}
