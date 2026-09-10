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
    apply!(ai.context_window_tokens);
    apply!(ai.show_thinking);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.candidates);
    apply!(run.query_timeout_seconds);
    apply!(jobs.wall_clock_seconds);
    apply!(jobs.tokens_per_endpoint);
    apply!(jobs.turns);
    apply!(jobs.tool_calls);
    apply!(output.format);
    apply!(output.color);
    apply!(ui.theme);
    apply!(memory.mode);
    apply!(memory.max_contracts);
    apply!(memory.max_claims_per_contract);
    apply!(memory.max_context_bytes);
    // `[[ai.endpoints]]` is a set of named entries, not a scalar: a layer's
    // entry for an existing name overlays that endpoint's fields (absent
    // field = leave the lower layer's value, matching `apply!`), a new name
    // is added. The trust boundary in `revert_untrusted` decides which of
    // those overlays are permitted.
    for endpoint in &layer.ai.endpoints {
        match base
            .ai
            .endpoints
            .iter_mut()
            .find(|existing| existing.name == endpoint.name)
        {
            Some(existing) => {
                if endpoint.provider.is_some() {
                    existing.provider = endpoint.provider;
                }
                if endpoint.model.is_some() {
                    existing.model = endpoint.model.clone();
                }
                if endpoint.base_url.is_some() {
                    existing.base_url = endpoint.base_url.clone();
                }
                if endpoint.api_key.is_some() {
                    existing.api_key = endpoint.api_key.clone();
                }
            }
            None => base.ai.endpoints.push(endpoint.clone()),
        }
    }
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
        &mut file.run.candidates,
        env,
        "SAYA_CANDIDATES",
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
    if cli.candidates.is_some() {
        file.run.candidates = cli.candidates;
    }
    if cli.show_thinking.is_some() {
        file.ai.show_thinking = cli.show_thinking;
    }
    if cli.theme.is_some() {
        file.ui.theme = cli.theme;
    }
}

/// The values of security-critical settings captured after the user layer
/// merges. Project layers may not change them: a repository's
/// `.saya/config.toml` is untrusted input, and these settings decide
/// where the API key is sent, whether rows leave the machine, and whether
/// engine-level read-only enforcement stays on.
///
/// `ai.show_thinking` is deliberately not on this list. It renders locally, to
/// the person who already sees the answer, and cannot exfiltrate anything the
/// answer does not already show — so it is an ordinary setting the project
/// layer may set without `--trust-project-config`. Adding a setting here is
/// a deliberate decision, not an oversight; this is that decision recorded
/// next to the list it joins.
///
/// `[[ai.endpoints]]` is protected per entry, not as a whole: the set of
/// *names* itself (the project layer may not add an endpoint the trusted
/// layers did not declare — an unnamed endpoint is still an attacker-chosen
/// destination), and per existing endpoint exactly the two fields that
/// redirect traffic or inject a credential, `base_url` and `api_key`. Those
/// are the whole point of the boundary: `base_url` decides where the request
/// goes and `api_key` decides which credential authenticates it, so either
/// one landing in a repository-controlled config points the user's model
/// traffic — and their key — at an attacker. An endpoint's `provider` and
/// `model` stay ordinary settings, like `[ai] model`: they name which model
/// answers, not where the request goes or what it authenticates with.
pub(crate) struct ProtectedSettings {
    ai_base_url: Option<String>,
    ai_api_key: Option<SecretRef>,
    ai_allow_data_sharing: Option<bool>,
    run_read_only: Option<bool>,
    /// The declared endpoint names, with each one's protected fields. Keyed
    /// by name so a project-layer entry is classified as either an override
    /// of a known endpoint or an addition.
    endpoints: BTreeMap<String, ProtectedEndpoint>,
}

#[derive(Clone)]
struct ProtectedEndpoint {
    base_url: Option<String>,
    api_key: Option<SecretRef>,
}

pub(crate) fn snapshot_protected(file: &ConfigFile) -> ProtectedSettings {
    ProtectedSettings {
        ai_base_url: file.ai.base_url.clone(),
        ai_api_key: file.ai.api_key.clone(),
        ai_allow_data_sharing: file.ai.allow_data_sharing,
        run_read_only: file.run.read_only,
        endpoints: file
            .ai
            .endpoints
            .iter()
            .map(|endpoint| {
                (
                    endpoint.name.clone(),
                    ProtectedEndpoint {
                        base_url: endpoint.base_url.clone(),
                        api_key: endpoint.api_key.clone(),
                    },
                )
            })
            .collect(),
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
    // Endpoints the untrusted layer added wholesale: a name the trusted
    // layers never declared. Removed entirely and reported by name — a bare
    // `ai.endpoints` would not tell the user what to fix.
    let mut added = Vec::new();
    file.ai.endpoints.retain(|endpoint| {
        let declared = before.endpoints.contains_key(&endpoint.name);
        if !declared {
            added.push(format!("ai.endpoints[{:?}]", endpoint.name));
        }
        declared
    });
    ignored.append(&mut added);
    // An existing endpoint reverts field-wise, so the report names which
    // endpoint was touched and which protected field was retargeted.
    for endpoint in &mut file.ai.endpoints {
        let Some(protected) = before.endpoints.get(&endpoint.name) else {
            continue;
        };
        if endpoint.base_url != protected.base_url {
            endpoint.base_url = protected.base_url.clone();
            ignored.push(format!("ai.endpoints[{:?}].base_url", endpoint.name));
        }
        if endpoint.api_key != protected.api_key {
            endpoint.api_key = protected.api_key.clone();
            ignored.push(format!("ai.endpoints[{:?}].api_key", endpoint.name));
        }
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
