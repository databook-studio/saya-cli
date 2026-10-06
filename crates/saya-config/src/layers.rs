use std::collections::BTreeMap;

use crate::{
    AiProvider, CliOverrides, CompactionMode, ConfigError, ConfigFile, OutputFormat,
    provenance::{Recorder, SettingSource, record_endpoint_fields},
};
use saya_types::SecretRef;

pub(crate) fn merge(
    base: &mut ConfigFile,
    layer: &ConfigFile,
    recorder: &mut Recorder,
    source: SettingSource,
) {
    macro_rules! apply { ($($path:ident).+) => { if layer.$($path).+.is_some() { base.$($path).+ = layer.$($path).+.clone(); recorder.record(stringify!($($path).+), source.clone()); } }; }
    apply!(default_profile);
    apply!(ai.provider);
    apply!(ai.model);
    apply!(ai.base_url);
    apply!(ai.allow_data_sharing);
    // The API key records the winning reference's redacted label, not just
    // the layer — the reference is the only display-safe fact about a secret.
    if let Some(secret) = layer.ai.api_key.as_ref() {
        base.ai.api_key = Some(secret.clone());
        recorder.record_secret("ai.api_key", source.clone(), secret);
    }
    apply!(ai.temperature);
    apply!(ai.timeout_seconds);
    apply!(ai.idle_timeout_seconds);
    apply!(ai.max_output_tokens);
    apply!(ai.retry_delays_ms);
    apply!(ai.context_byte_budget);
    apply!(ai.investigation_max_logical_answering_requests);
    apply!(ai.investigation_max_requested_tool_calls);
    apply!(ai.investigation_max_elapsed_seconds);
    apply!(ai.investigation_max_known_reported_tokens);
    apply!(ai.context_window_tokens);
    apply!(ai.show_thinking);
    apply!(ai.compaction);
    apply!(run.read_only);
    apply!(run.max_rows);
    apply!(run.max_iterations);
    apply!(run.candidates);
    apply!(run.query_timeout_seconds);
    apply!(jobs.wall_clock_seconds);
    apply!(jobs.tokens_per_endpoint);
    apply!(jobs.turns);
    apply!(jobs.tool_calls);
    apply!(jobs.fetch);
    apply!(jobs.runner);
    apply!(jobs.interpreter);
    // `[host_commands]` merges like any ordinary user-layer section here —
    // the project layer never reaches this merge with the section set (the
    // typed refusal in `resolve.rs` fires first).
    if !layer.host_commands.pass_env.is_empty() {
        base.host_commands.pass_env = layer.host_commands.pass_env.clone();
        recorder.record("host_commands.pass_env", source.clone());
    }
    if layer.host_commands.timeout_seconds.is_some() {
        base.host_commands.timeout_seconds = layer.host_commands.timeout_seconds;
        recorder.record("host_commands.timeout_seconds", source.clone());
    }
    // `[session_commands] deny` merges like any ordinary user-layer section
    // here — the project layer never reaches this merge with the section set
    // (the typed refusal in `resolve.rs` fires first).
    if !layer.session_commands.deny.is_empty() {
        base.session_commands.deny = layer.session_commands.deny.clone();
        recorder.record("session_commands.deny", source.clone());
    }
    apply!(output.format);
    apply!(output.color);
    apply!(ui.theme);
    apply!(memory.mode);
    apply!(memory.max_contracts);
    apply!(memory.max_claims_per_contract);
    apply!(memory.max_context_bytes);
    // `[instructions]` merges per field like the other scalar sections; the
    // trust boundary in `revert_untrusted` reverts all five keys, since they
    // decide what instruction context the model is assembled from.
    apply!(instructions.project);
    apply!(instructions.personal);
    apply!(instructions.project_max_bytes);
    apply!(instructions.personal_max_bytes);
    apply!(instructions.total_max_bytes);
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
        record_endpoint_fields(recorder, &endpoint.name, endpoint, source.clone());
    }
}

pub(crate) fn apply_env(
    file: &mut ConfigFile,
    env: &BTreeMap<String, String>,
    recorder: &mut Recorder,
    source: &dyn Fn(&str) -> SettingSource,
) -> Result<(), ConfigError> {
    apply_string(
        &mut file.ai.model,
        env,
        "SAYA_AI_MODEL",
        "ai.model",
        recorder,
        source,
    );
    apply_string(
        &mut file.ai.base_url,
        env,
        "SAYA_AI_BASE_URL",
        "ai.base_url",
        recorder,
        source,
    );
    apply_secret(
        &mut file.ai.api_key,
        env,
        "SAYA_AI_API_KEY",
        "ai.api_key",
        recorder,
        source,
    );
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_AI_PROVIDER",
        "ai.provider",
        recorder,
        source,
        AiProvider::parse,
    )?;
    apply_string(
        &mut file.ai.model,
        env,
        "SAYA_MODEL",
        "ai.model",
        recorder,
        source,
    );
    apply_string(
        &mut file.ai.base_url,
        env,
        "SAYA_PROVIDER_BASE_URL",
        "ai.base_url",
        recorder,
        source,
    );
    apply_secret(
        &mut file.ai.api_key,
        env,
        "SAYA_API_KEY",
        "ai.api_key",
        recorder,
        source,
    );
    apply_parsed(
        &mut file.ai.provider,
        env,
        "SAYA_PROVIDER",
        "ai.provider",
        recorder,
        source,
        AiProvider::parse,
    )?;
    apply_parsed(
        &mut file.ai.allow_data_sharing,
        env,
        "SAYA_ALLOW_DATA_SHARING",
        "ai.allow_data_sharing",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.ai.compaction,
        env,
        "SAYA_COMPACTION",
        "ai.compaction",
        recorder,
        source,
        CompactionMode::parse,
    )?;
    apply_parsed(
        &mut file.run.read_only,
        env,
        "SAYA_READ_ONLY",
        "run.read_only",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.max_rows,
        env,
        "SAYA_MAX_ROWS",
        "run.max_rows",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.max_iterations,
        env,
        "SAYA_MAX_ITERATIONS",
        "run.max_iterations",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.candidates,
        env,
        "SAYA_CANDIDATES",
        "run.candidates",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.run.query_timeout_seconds,
        env,
        "SAYA_QUERY_TIMEOUT_SECONDS",
        "run.query_timeout_seconds",
        recorder,
        source,
        parse_value,
    )?;
    apply_parsed(
        &mut file.output.format,
        env,
        "SAYA_OUTPUT_FORMAT",
        "output.format",
        recorder,
        source,
        OutputFormat::parse,
    )?;
    Ok(())
}

pub(crate) fn apply_cli(file: &mut ConfigFile, cli: &CliOverrides, recorder: &mut Recorder) {
    if cli.provider.is_some() {
        file.ai.provider = cli.provider;
        recorder.record(
            "ai.provider",
            SettingSource::Cli {
                flag: "--provider".into(),
            },
        );
    }
    if cli.model.is_some() {
        file.ai.model = cli.model.clone();
        recorder.record(
            "ai.model",
            SettingSource::Cli {
                flag: "--model".into(),
            },
        );
    }
    if cli.allow_data_sharing.is_some() {
        file.ai.allow_data_sharing = cli.allow_data_sharing;
        recorder.record(
            "ai.allow_data_sharing",
            SettingSource::Cli {
                flag: "--allow-data-sharing".into(),
            },
        );
    }
    if cli.max_rows.is_some() {
        file.run.max_rows = cli.max_rows;
        recorder.record(
            "run.max_rows",
            SettingSource::Cli {
                flag: "--max-rows".into(),
            },
        );
    }
    if cli.candidates.is_some() {
        file.run.candidates = cli.candidates;
        recorder.record(
            "run.candidates",
            SettingSource::Cli {
                flag: "--candidates".into(),
            },
        );
    }
    if cli.show_thinking.is_some() {
        file.ai.show_thinking = cli.show_thinking;
        recorder.record(
            "ai.show_thinking",
            SettingSource::Cli {
                flag: "--show-thinking".into(),
            },
        );
    }
    if cli.theme.is_some() {
        file.ui.theme = cli.theme;
        recorder.record(
            "ui.theme",
            SettingSource::Cli {
                flag: "--theme".into(),
            },
        );
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
///
/// `[jobs.interpreter]` is protected as a whole sub-table. The
/// interpreter approval's argument: the user typing
/// `--allow interpreter:python3` is approving *the python3 of their
/// toolchain*, but an interpreter's staging inputs — which binary answers
/// the name, which support trees become readable — decide what actually
/// executes under it. A repository-controlled config that answers
/// "python3" with a binary of its own choosing turns the user's typed
/// approval into approval of bytes the user never saw — the `ai.base_url`
/// class of decision: the flag the user types is redirected by an untrusted
/// file. And an interpreter the trusted layers never declared is still an
/// attacker-chosen program, the endpoint-name precedent applied to
/// programs. The whole sub-table is snapshotted and reverted, reported by
/// its dotted name, so the user is told what was ignored. A compiled
/// program's behaviour is fixed and reviewable before the run, which is why
/// `[jobs.runner]` — its sibling — stays an ordinary setting: an interpreter
/// approval voids a security contract, and it would be void against bytes
/// the staging config chose.
pub(crate) struct ProtectedSettings {
    ai_base_url: Option<String>,
    ai_api_key: Option<SecretRef>,
    ai_allow_data_sharing: Option<bool>,
    ai_investigation_max_logical_answering_requests: Option<toml::Value>,
    ai_investigation_max_requested_tool_calls: Option<toml::Value>,
    ai_investigation_max_elapsed_seconds: Option<toml::Value>,
    ai_investigation_max_known_reported_tokens: Option<toml::Value>,
    run_read_only: Option<bool>,
    /// The `[instructions]` keys, protected one field at a time: they decide
    /// which instruction files are read and how many bytes of them the
    /// model's context is assembled from, so an untrusted layer must not
    /// disable a source, raise a ceiling, or (there is no path key to abuse)
    /// steer any of it.
    instructions_project: Option<bool>,
    instructions_personal: Option<bool>,
    instructions_project_max_bytes: Option<usize>,
    instructions_personal_max_bytes: Option<usize>,
    instructions_total_max_bytes: Option<usize>,
    /// The whole `[jobs.interpreter]` sub-table as the trusted layers
    /// declared it: which interpreters may ever resolve, and by extension
    /// which bytes answer them.
    jobs_interpreter: Option<crate::model::InterpreterJobsFile>,
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
        ai_investigation_max_logical_answering_requests: file
            .ai
            .investigation_max_logical_answering_requests
            .clone(),
        ai_investigation_max_requested_tool_calls: file
            .ai
            .investigation_max_requested_tool_calls
            .clone(),
        ai_investigation_max_elapsed_seconds: file.ai.investigation_max_elapsed_seconds.clone(),
        ai_investigation_max_known_reported_tokens: file
            .ai
            .investigation_max_known_reported_tokens
            .clone(),
        run_read_only: file.run.read_only,
        instructions_project: file.instructions.project,
        instructions_personal: file.instructions.personal,
        instructions_project_max_bytes: file.instructions.project_max_bytes,
        instructions_personal_max_bytes: file.instructions.personal_max_bytes,
        instructions_total_max_bytes: file.instructions.total_max_bytes,
        jobs_interpreter: file.jobs.interpreter.clone(),
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
    revert_ai_budget(
        &mut file.ai.investigation_max_logical_answering_requests,
        &before.ai_investigation_max_logical_answering_requests,
        "investigation_max_logical_answering_requests",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_requested_tool_calls,
        &before.ai_investigation_max_requested_tool_calls,
        "investigation_max_requested_tool_calls",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_elapsed_seconds,
        &before.ai_investigation_max_elapsed_seconds,
        "investigation_max_elapsed_seconds",
        &mut ignored,
    );
    revert_ai_budget(
        &mut file.ai.investigation_max_known_reported_tokens,
        &before.ai_investigation_max_known_reported_tokens,
        "investigation_max_known_reported_tokens",
        &mut ignored,
    );
    if file.run.read_only != before.run_read_only {
        file.run.read_only = before.run_read_only;
        ignored.push("run.read_only".into());
    }
    // Instruction enablement and limits are protected per field: which
    // instruction files load, and how many bytes of them may reach the
    // model's context, is trusted-layer business. Reported by dotted name so
    // the user is told what was ignored.
    revert_instruction(
        &mut file.instructions.project,
        &before.instructions_project,
        "project",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.personal,
        &before.instructions_personal,
        "personal",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.project_max_bytes,
        &before.instructions_project_max_bytes,
        "project_max_bytes",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.personal_max_bytes,
        &before.instructions_personal_max_bytes,
        "personal_max_bytes",
        &mut ignored,
    );
    revert_instruction(
        &mut file.instructions.total_max_bytes,
        &before.instructions_total_max_bytes,
        "total_max_bytes",
        &mut ignored,
    );
    // The interpreter universe is protected as a whole sub-table: which
    // interpreters may ever resolve, and by extension which bytes answer
    // them, is trusted-layer business. Reported by its dotted name so the
    // user is told what was ignored.
    if file.jobs.interpreter != before.jobs_interpreter {
        file.jobs.interpreter = before.jobs_interpreter.clone();
        ignored.push("jobs.interpreter".into());
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
