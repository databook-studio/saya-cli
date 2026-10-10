use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use crate::{
    AiProvider, ColorChoice, ConfigError, ConfigFile, OutputFormat, ResolutionInput, ThemeChoice,
    endpoints::{ResolvedEndpoint, require_unique_endpoints, resolve_endpoints},
    jobs::{ResolvedJobs, require_max_iterations},
    layers::{apply_cli, apply_env, merge, revert_untrusted, snapshot_protected},
    memory::ResolvedMemory,
    profile_env::overlay_database_environment,
    provenance::{
        Recorder, SettingProvenance, SettingSource, apply_coding_defaults,
        apply_user_facing_defaults,
    },
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

/// Default `[run] max_iterations`: a stored setting with no behavioural
/// reader — `[jobs] turns` is opt-in, and unset means unlimited. Kept
/// parsed and range-checked so existing configs keep resolving; nothing
/// consumes it.
const DEFAULT_MAX_ITERATIONS: usize = 12;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub profile_name: Option<String>,
    pub profile: Option<DatabaseProfile>,
    pub ai: ResolvedAi,
    /// Trusted opt-in coding admission settings. They remain separate from
    /// authority: enabled configuration never grants a capability by itself.
    pub coding: crate::ResolvedCoding,
    /// Validated ceilings for one interactive investigation. These are a
    /// configuration contract; runtime enforcement is composed separately.
    pub investigation_budgets: crate::ResolvedInvestigationBudgets,
    pub max_rows: usize,
    pub read_only: bool,
    pub max_iterations: usize,
    /// Independent agent attempts per question. Defaults to `1` (today's
    /// single-run behaviour); each extra candidate is a full additional agent
    /// run. Bounded to `1..=16` at resolve time. Read by the `ask` command,
    /// which runs one attempt per candidate and votes on their nominated SQL
    /// (`crates/saya-cli/src/agent/candidates`).
    pub candidates: usize,
    /// The default budgets a run is declared with: `[jobs]` resolved per
    /// dimension — a ceiling left unset is unlimited at the contract level.
    /// The engine layers RunSpec and step budgets over these; there is
    /// deliberately no environment input to any of it (plan G3).
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
    /// The resolved `[instructions]` sources and their validated byte
    /// limits: which instruction files load and how many bytes of them may
    /// reach the context. Enablement and limits are protected settings, so
    /// these always reflect the trusted layers.
    pub instructions: crate::ResolvedInstructions,
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
    /// For every user-facing effective setting, the configuration layer
    /// that supplied it — value-free provenance where a secret appears only
    /// as its redacted reference label. Pure observation of the merge: the
    /// precedence, values, and errors are exactly what they were before.
    pub provenance: SettingProvenance,
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
    /// True when `max_output_tokens` is the built-in default because no layer
    /// stated it. Data, not presentation: callers that report the ceiling
    /// (doctor) need the distinction, and only resolution can see it.
    pub max_output_tokens_is_default: bool,
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
    /// How context compaction behaves once the window fills. `auto` (the
    /// default) summarises older turns on crossing the compact threshold;
    /// `manual` keeps `/compact` working and never fires on its own; `off`
    /// additionally silences the 70% warning.
    pub compaction: crate::CompactionMode,
}

#[path = "resolve/resolution.rs"]
mod resolution;
#[path = "resolve/validation.rs"]
mod validation;

pub use resolution::resolve;
use validation::{
    require_candidates, require_context_byte_budget, require_context_window_tokens,
    require_retry_delays,
};
