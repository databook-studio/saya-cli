use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use crate::{
    AiProvider, ColorChoice, ConfigError, ConfigFile, OutputFormat, ResolutionInput, ThemeChoice,
    endpoints::{ResolvedEndpoint, require_unique_endpoints, resolve_endpoints},
    jobs::{ResolvedJobs, require_max_iterations},
    layers::{apply_cli, apply_env, merge, revert_untrusted, snapshot_protected},
    memory::ResolvedMemory,
    profile_env::overlay_database_environment,
};

const DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

/// Default conversation byte budget: the 256 KiB the agent loop used before
/// this setting existed (a user with no setting changes nothing).
const DEFAULT_CONTEXT_BYTE_BUDGET: usize = 256 * 1024;

/// Smallest accepted `[ai] context_byte_budget`. Below this the budget is too
/// small to hold a system prompt and a single turn, so the loop would trim away
/// useful context on every turn — a budget of 0 trims the conversation to
/// nothing. Matched against the existing `[memory]` range-check style rather
/// than a silent clamp. No upper bound: a user with a large
/// context window may raise it freely, which is the point of making it settable.
const MIN_CONTEXT_BYTE_BUDGET: usize = 1024;

/// Default provider retry backoff in milliseconds: three sleeps before the
/// provider gives up. A user who sets nothing gets these.
const DEFAULT_RETRY_DELAYS_MS: &[u64] = &[250, 500, 1000];

/// Most provider retries a config-supplied schedule may request. Each retry
/// repeats a full failing request, so an unbounded list turns one provider
/// failure into many; this bounds that cost while leaving room to widen beyond
/// the three-entry default for a slow or rate-limited gateway. Each sleep is
/// also capped at 60s by the provider HTTP layer, so worst-case backoff
/// sleeping is this count times 60s.
const MAX_RETRY_DELAYS: usize = 8;

/// Default independent agent attempts per question. `1` is today's single-run
/// behaviour, so a user who sets nothing changes nothing in cost or latency.
const DEFAULT_CANDIDATES: usize = 1;

/// Smallest accepted `[run] candidates`. Zero is meaningless — zero attempts
/// answer nothing — so it is rejected rather than silently clamped to one.
const MIN_CANDIDATES: usize = 1;

/// Most independent agent attempts a config may request. Each candidate is a
/// full agent run (model calls and database queries), so an unbounded value
/// would let a typo start hundreds of runs and multiply a user's bill. Sixteen
/// leaves room to opt into a wider search while keeping the worst case bounded.
const MAX_CANDIDATES: usize = 16;

/// Default `[run] max_iterations`, and through it the default turn ceiling
/// for a run's episodes: a run with nothing declared stops after twelve
/// provider turns and salvages the best answer from the work done.
const DEFAULT_MAX_ITERATIONS: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub profile_name: Option<String>,
    pub profile: Option<DatabaseProfile>,
    pub ai: ResolvedAi,
    pub max_rows: usize,
    pub read_only: bool,
    pub max_iterations: usize,
    /// Independent agent attempts per question. Defaults to `1` (today's
    /// single-run behaviour); each extra candidate is a full additional agent
    /// run. Bounded to `1..=16` at resolve time. Read by the `ask` command,
    /// which runs one attempt per candidate and votes on their nominated SQL
    /// (`crates/saya-cli/src/agent/candidates`).
    pub candidates: usize,
    /// The default budgets a run is declared with: `[jobs]` resolved against
    /// `[run] max_iterations`, whose turn ceiling falls back to
    /// `max_iterations` (plan G2 — that knob's first behavioural reader).
    /// The engine layers RunSpec and step budgets over these per dimension;
    /// there is deliberately no environment input to any of it (plan G3).
    pub jobs: ResolvedJobs,
    /// The `[host_commands]` shaping: user-layer `pass_env` and
    /// the per-call ceiling. The project layer may never state it (typed
    /// resolve error) — see `HostCommandsFromProject`.
    pub host_commands: crate::jobs::ResolvedHostCommands,
    /// The user-layer `[session_commands] deny` list: bare program names
    /// every session door refuses before grant, prompt, and bypass. The
    /// project layer may never state it (typed resolve error).
    pub session_deny: crate::jobs::ResolvedSessionDeny,
    pub query_timeout_seconds: u64,
    pub output_format: OutputFormat,
    pub output_color: ColorChoice,
    pub ui_theme: ThemeChoice,
    pub memory: ResolvedMemory,
    /// Security-critical setting names (`ai.base_url`, `run.read_only`, …)
    /// that the project layer tried to override and were ignored. Empty when
    /// the project layer is trusted or set none of them.
    pub ignored_project_overrides: Vec<String>,
    /// The named AI endpoints a run's roles bind to, keyed by run-scoped
    /// name. Always contains `orchestrator`: the plain `[ai]` block whenever
    /// no `[[ai.endpoints]]` entry is declared with that name, so an existing
    /// config resolves the same pool it did before endpoints existed. An
    /// endpoint's `api_key` stays a reference here; values are resolved only
    /// at request time.
    pub endpoints: BTreeMap<String, ResolvedEndpoint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAi {
    pub provider: AiProvider,
    pub model: String,
    pub base_url: Option<String>,
    pub api_key: Option<saya_types::SecretRef>,
    pub allow_data_sharing: bool,
    /// Sampling temperature for the LLM (lower = more concise/deterministic).
    pub temperature: f32,
    /// Budget for request establishment / non-streaming responses.
    pub timeout_seconds: u64,
    /// Maximum silence between stream chunks before the provider is stalled.
    pub idle_timeout_seconds: u64,
    /// Per-response output-token ceiling requested from the provider.
    pub max_output_tokens: u32,
    /// Provider retry backoff in milliseconds, tried in order before the
    /// provider gives up. An empty list means one attempt with no sleeps.
    pub retry_delays_ms: Vec<u64>,
    /// Ceiling on the approximate byte size of the conversation the agent loop
    /// assembles. The loop trims under it instead of aborting, so a user on a
    /// model with a large context window can raise it to keep more history.
    pub context_byte_budget: usize,
    /// The model's context window in tokens, as the user declared it in
    /// `[ai] context_window_tokens`. `None` is the normal case: the user did
    /// not declare one, and the answer is whatever the built-in table says for
    /// `model` (or nothing, for a model it does not know). Absent is not zero —
    /// a declared value is a fact about this deployment; an undeclared one is
    /// not a fact at all.
    pub context_window_tokens: Option<u64>,
    /// Show the model's chain-of-thought in the transcript. Off by default;
    /// display only — reasoning is never persisted regardless of this setting.
    pub show_thinking: bool,
}

pub fn resolve(input: ResolutionInput) -> Result<ResolvedConfig, ConfigError> {
    let mut file = ConfigFile::default();
    if let Some(user) = input.user.as_ref() {
        require_unique_endpoints(&user.ai.endpoints)?;
        merge(&mut file, user);
    }
    let protected = snapshot_protected(&file);
    if let Some(project) = input.project.as_ref() {
        require_unique_endpoints(&project.ai.endpoints)?;
        // A project-layer `[host_commands]` is a hard refusal — not a
        // revert: a model-writable file must never shape unsandboxed
        // execution (not enable it, not widen its timeout, not name its
        // env), and `--trust-project-config` does not unlock it.
        if !project.host_commands.pass_env.is_empty()
            || project.host_commands.timeout_seconds.is_some()
        {
            return Err(ConfigError::HostCommandsFromProject);
        }
        // A project-layer `[session_commands]` is a hard refusal — not a
        // revert: a model-writable deny could name every `[jobs.runner]`
        // allow entry, herding the session's work onto the unsandboxed lane.
        if !project.session_commands.deny.is_empty() {
            return Err(ConfigError::SessionCommandsFromProject);
        }
        merge(&mut file, project);
    }
    let ignored_project_overrides = if input.cli.trust_project_config {
        Vec::new()
    } else {
        revert_untrusted(&mut file, &protected)
    };
    apply_env(&mut file, &input.env_file)?;
    apply_env(&mut file, &input.process_env)?;
    apply_cli(&mut file, &input.cli);
    let selected = input
        .cli
        .profile
        .or_else(|| input.process_env.get("SAYA_PROFILE").cloned())
        .or(file.default_profile.clone())
        .or_else(|| {
            (input.connections.profiles.len() == 1)
                .then(|| input.connections.profiles.keys().next().cloned())
                .flatten()
        });
    if selected.is_none() && input.connections.profiles.len() > 1 {
        return Err(ConfigError::MissingProfile);
    }
    let environment_profile = input.env_file.contains_key("SAYA_DB_TYPE")
        || input.process_env.contains_key("SAYA_DB_TYPE");
    let profile = selected
        .as_ref()
        .map(|name| match input.connections.profiles.get(name).cloned() {
            Some(profile) => Ok(Some(profile)),
            None if environment_profile => Ok(None),
            None => Err(ConfigError::UnknownProfile(name.clone())),
        })
        .transpose()?
        .flatten();
    let profile = overlay_database_environment(profile, &input.env_file, &input.process_env)?;
    let memory = crate::memory::resolve(&file.memory)?;
    let context_byte_budget = file
        .ai
        .context_byte_budget
        .unwrap_or(DEFAULT_CONTEXT_BYTE_BUDGET);
    require_context_byte_budget(context_byte_budget)?;
    require_context_window_tokens(file.ai.context_window_tokens)?;
    let retry_delays_ms = file
        .ai
        .retry_delays_ms
        .clone()
        .unwrap_or_else(|| DEFAULT_RETRY_DELAYS_MS.to_vec());
    require_retry_delays(&retry_delays_ms)?;
    let candidates = file.run.candidates.unwrap_or(DEFAULT_CANDIDATES);
    require_candidates(candidates)?;
    let max_iterations = file.run.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS);
    require_max_iterations(max_iterations)?;
    let jobs = crate::jobs::resolve(&file.jobs, max_iterations as u64)?;
    let host_commands = crate::jobs::resolve_host_commands(file.host_commands.clone())?;
    let session_deny = crate::jobs::resolve_session_deny(file.session_commands.clone())?;
    let ai = ResolvedAi {
        provider: file.ai.provider.unwrap_or(AiProvider::Ollama),
        model: file
            .ai
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.into()),
        // Cloned, not moved: the endpoint map inherits these same fields as
        // its fallback, and the file is read again by `resolve_endpoints`.
        base_url: file.ai.base_url.clone(),
        api_key: file.ai.api_key.clone(),
        allow_data_sharing: file.ai.allow_data_sharing.unwrap_or(false),
        temperature: file.ai.temperature.unwrap_or(0.1),
        timeout_seconds: file.ai.timeout_seconds.unwrap_or(60),
        idle_timeout_seconds: file.ai.idle_timeout_seconds.unwrap_or(90),
        max_output_tokens: file.ai.max_output_tokens.unwrap_or(4096),
        context_byte_budget,
        context_window_tokens: file.ai.context_window_tokens,
        show_thinking: file.ai.show_thinking.unwrap_or(false),
        retry_delays_ms,
    };
    let endpoints = resolve_endpoints(&file.ai, &ai)?;
    Ok(ResolvedConfig {
        profile_name: selected,
        profile,
        ai,
        max_rows: file.run.max_rows.unwrap_or(1000),
        read_only: file.run.read_only.unwrap_or(true),
        max_iterations,
        candidates,
        jobs,
        host_commands,
        session_deny,
        query_timeout_seconds: file.run.query_timeout_seconds.unwrap_or(60),
        output_format: file.output.format.unwrap_or(OutputFormat::Text),
        output_color: file.output.color.unwrap_or(ColorChoice::Auto),
        ui_theme: file.ui.theme.unwrap_or(ThemeChoice::Auto),
        memory,
        ignored_project_overrides,
        endpoints,
    })
}

/// Rejects an `[ai] context_byte_budget` below the floor with a typed error
///, matching the `[memory]` range-check style. The upper end is
/// unbounded: a user may raise the budget to fit a larger context window, which
/// is the reason the setting exists, so no ceiling is enforced here.
fn require_context_byte_budget(value: usize) -> Result<(), ConfigError> {
    if value >= MIN_CONTEXT_BYTE_BUDGET {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field: "context_byte_budget",
            value,
            min: MIN_CONTEXT_BYTE_BUDGET,
        })
    }
}

/// Rejects a declared `[ai] context_window_tokens` of zero. A window of zero
/// tokens is a typo, not a deployment: no model turns every prompt into a
/// refusal. There is no upper bound — a user declaring their gateway's window
/// is stating a fact saya has no other way to learn, and the largest published
/// window today is a few million tokens, so any plausible ceiling would
/// outlive its reason.
fn require_context_window_tokens(value: Option<u64>) -> Result<(), ConfigError> {
    if value.is_none_or(|tokens| tokens > 0) {
        Ok(())
    } else {
        Err(ConfigError::SettingBelowMinimum {
            field: "context_window_tokens",
            value: value.unwrap_or_default() as usize,
            min: 1,
        })
    }
}

/// Rejects an `[ai] retry_delays_ms` schedule longer than `MAX_RETRY_DELAYS`.
/// An empty list is allowed — it means "do not retry" (one attempt, no
/// sleeps), which is a valid choice. Sibling to `require_context_byte_budget`
/// in style: a typed error at resolve time rather than a silent clamp at the
/// point of use.
fn require_retry_delays(value: &[u64]) -> Result<(), ConfigError> {
    if value.len() <= MAX_RETRY_DELAYS {
        Ok(())
    } else {
        Err(ConfigError::SettingAboveMaximum {
            field: "retry_delays_ms",
            value: value.len(),
            max: MAX_RETRY_DELAYS,
        })
    }
}

/// Rejects a `[run] candidates` outside `MIN_CANDIDATES..=MAX_CANDIDATES`.
/// Zero is meaningless (zero attempts answer nothing) and an unbounded value
/// would let a typo start hundreds of full agent runs, multiplying a user's
/// model spend. The accepted range is reported, not just one bound, matching
/// the `[memory]` range-check style. Sibling to `require_retry_delays`.
fn require_candidates(value: usize) -> Result<(), ConfigError> {
    if (MIN_CANDIDATES..=MAX_CANDIDATES).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::SettingOutOfRange {
            field: "candidates",
            value,
            min: MIN_CANDIDATES,
            max: MAX_CANDIDATES,
        })
    }
}
