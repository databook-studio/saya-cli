use std::collections::BTreeMap;

use crate::{AiProvider, CliOverrides, ConfigError, ConfigFile, OutputFormat};
use saya_types::SecretRef;

pub(crate) fn merge(base: &mut ConfigFile, layer: &ConfigFile) {
    macro_rules! apply { ($($path:ident).+) => { if layer.$($path).+.is_some() { base.$($path).+ = layer.$($path).+.clone(); } }; }
    apply!(default_profile);
    apply!(ai.provider);
    apply!(ai.model);
    apply!(ai.base_url);
    apply!(ai.allow_data_sharing);
    apply!(ai.api_key);
    apply!(ai.temperature);
    apply!(ai.timeout_seconds);
    apply!(ai.idle_timeout_seconds);
    apply!(ai.max_output_tokens);
    apply!(ai.retry_delays_ms);
    apply!(ai.context_byte_budget);
    apply!(ai.show_thinking);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.query_timeout_seconds);
    apply!(output.format);
    apply!(output.color);
    apply!(memory.mode);
    apply!(memory.max_contracts);
    apply!(memory.max_claims_per_contract);
    apply!(memory.max_context_bytes);
}

pub(crate) fn apply_env(
    file: &mut ConfigFile,
    env: &BTreeMap<String, String>,
) -> Result<(), ConfigError> {
    apply_string(&mut file.ai.model, env, "SAYA_AI_MODEL");
    apply_string(&mut file.ai.base_url, env, "SAYA_AI_BASE_URL");
    apply_secret(&mut file.ai.api_key, env, "SAYA_AI_API_KEY");
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_AI_PROVIDER",
        AiProvider::parse,
    )?;
    apply_string(&mut file.ai.model, env, "SAYA_MODEL");
    apply_string(&mut file.ai.base_url, env, "SAYA_PROVIDER_BASE_URL");
    apply_secret(&mut file.ai.api_key, env, "SAYA_API_KEY");
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_PROVIDER",
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
    if cli.show_thinking.is_some() {
        file.ai.show_thinking = cli.show_thinking;
    }
}

/// The values of security-critical settings captured after the user layer
/// merges. Project layers may not change them: a repository's
/// `.saya/config.toml` is untrusted input, and these four settings decide
/// where the API key is sent, whether rows leave the machine, and whether
/// engine-level read-only enforcement stays on.
///
/// `ai.show_thinking` is deliberately not on this list. It renders locally, to
/// the person who already sees the answer, and cannot exfiltrate anything the
/// answer does not already show — so it is an ordinary setting the project
/// layer may set without `--trust-project-config`. Adding a fifth protected
/// setting would be a deliberate decision, not an oversight; this is that
/// decision recorded next to the list it would join.
pub(crate) struct ProtectedSettings {
    ai_base_url: Option<String>,
    ai_api_key: Option<SecretRef>,
    ai_allow_data_sharing: Option<bool>,
    run_read_only: Option<bool>,
}

pub(crate) fn snapshot_protected(file: &ConfigFile) -> ProtectedSettings {
    ProtectedSettings {
        ai_base_url: file.ai.base_url.clone(),
        ai_api_key: file.ai.api_key.clone(),
        ai_allow_data_sharing: file.ai.allow_data_sharing,
        run_read_only: file.run.read_only,
    }
}

/// Restores the protected settings to their pre-project values and returns
/// the dotted names the project layer tried (and failed) to override.
pub(crate) fn revert_untrusted(file: &mut ConfigFile, before: &ProtectedSettings) -> Vec<String> {
    let mut ignored = Vec::new();
    if file.ai.base_url != before.ai_base_url {
        file.ai.base_url = before.ai_base_url.clone();
        ignored.push("ai.base_url".into());
    }
    if file.ai.api_key != before.ai_api_key {
        file.ai.api_key = before.ai_api_key.clone();
        ignored.push("ai.api_key".into());
    }
    if file.ai.allow_data_sharing != before.ai_allow_data_sharing {
        file.ai.allow_data_sharing = before.ai_allow_data_sharing;
        ignored.push("ai.allow_data_sharing".into());
    }
    if file.run.read_only != before.run_read_only {
        file.run.read_only = before.run_read_only;
        ignored.push("run.read_only".into());
    }
    ignored
}

fn apply_string(target: &mut Option<String>, env: &BTreeMap<String, String>, name: &str) {
    if let Some(value) = env.get(name) {
        *target = Some(value.clone());
    }
}

fn apply_secret(
    target: &mut Option<saya_types::SecretRef>,
    env: &BTreeMap<String, String>,
    name: &str,
) {
    if env.contains_key(name) {
        *target = Some(saya_types::SecretRef::Env { env: name.into() });
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
