use std::collections::BTreeMap;

use saya_types::DatabaseProfile;

use crate::{
    AiProvider, ColorChoice, ConfigError, ConfigFile, OutputFormat, ResolutionInput, ThemeChoice,
    endpoints::{ResolvedEndpoint, require_unique_endpoints, resolve_endpoints},
    jobs::ResolvedJobs,
    layers::{apply_cli, apply_env, merge, revert_untrusted, snapshot_protected},
    memory::ResolvedMemory,
    profile_env::overlay_database_environment,
    provenance::{
        Recorder, SettingProvenance, SettingSource, apply_agents_defaults, apply_job_defaults,
        apply_tools_defaults, apply_user_facing_defaults,
    },
};

const DEFAULT_MODEL: &str = "qwen2.5-coder:14b";

/// Default conversation byte budget: the 256 KiB the agent loop used before
/// this setting existed (a user with no setting changes nothing).
const DEFAULT_CONTEXT_BYTE_BUDGET: usize = 256 * 1024;

/// Bytes per token assumed when deriving the conversation budget from a model
/// window. A rough four bytes per token is the conventional English-text
/// estimate; it is deliberately a named constant so the assumption is visible
/// rather than buried in the arithmetic.
const CONTEXT_BYTES_PER_TOKEN: u64 = 4;

/// Share of a window's estimated byte capacity kept as the conversation
/// budget, in percent. The remaining tenth absorbs tokenizer and estimate
/// error, so the derived budget never promises the whole window.
const DERIVED_BUDGET_PERCENT: u64 = 90;

/// Ceiling on a window-derived `[ai] context_byte_budget`: 4 MiB. Derivation
/// is bounded so a very large declared window cannot turn a typo into an
/// unbounded conversation. An explicitly set budget is unaffected — a user
/// who names a number gets exactly that number.
const MAX_DERIVED_CONTEXT_BYTE_BUDGET: usize = 4 * 1024 * 1024;

/// Derives the conversation byte budget from a known context window:
/// `window_tokens × 4 bytes × 90%`, clamped to the 256 KiB floor and the
/// 4 MiB ceiling. The floor keeps the historical behaviour on small windows;
/// the ceiling bounds the derivation.
fn derive_context_byte_budget(window_tokens: u64) -> usize {
    let bytes = window_tokens
        .saturating_mul(CONTEXT_BYTES_PER_TOKEN)
        .saturating_mul(DERIVED_BUDGET_PERCENT)
        / 100;
    usize::try_from(bytes)
        .unwrap_or(usize::MAX)
        .clamp(DEFAULT_CONTEXT_BYTE_BUDGET, MAX_DERIVED_CONTEXT_BYTE_BUDGET)
}

/// Smallest accepted `[ai] context_byte_budget`. Below this the budget is too
/// small to hold a system prompt and a single turn, so the loop would trim away
/// useful context on every turn — a budget of 0 trims the conversation to
/// nothing. Matched against the existing `[memory]` range-check style rather
/// than a silent clamp. No upper bound: a user with a large
/// context window may raise it freely, which is the point of making it settable.
const MIN_CONTEXT_BYTE_BUDGET: usize = 1024;

/// Default provider retry backoff in milliseconds: eight sleeps with
/// exponential growth (0.5, 1, 2, 4, 8, 16, 30, 30 s) before the provider
/// gives up. The last two entries stop growing at the provider layer's 30 s
/// per-delay ceiling. A user who sets nothing gets these; the list stays
/// overridable through `[ai] retry_delays_ms`.
const DEFAULT_RETRY_DELAYS_MS: &[u64] = &[500, 1000, 2000, 4000, 8000, 16000, 30000, 30000];

/// Default share of a known context window at which the agent loop compacts.
/// Half the window leaves room for the next request's prompt and output
/// without letting a long run reach the window itself.
pub const DEFAULT_COMPACTION_TRIGGER_PERCENT: u8 = 50;

/// Smallest accepted `[ai] compaction_trigger_percent`. Zero would compact a
/// conversation that holds nothing, on every turn.
const MIN_COMPACTION_TRIGGER_PERCENT: u8 = 1;

/// Largest accepted `[ai] compaction_trigger_percent`. Above the whole window
/// the trigger could never fire before the request itself was refused.
const MAX_COMPACTION_TRIGGER_PERCENT: u8 = 100;

/// Most provider retries a config-supplied schedule may request. Each retry
/// repeats a full failing request, so an unbounded list turns one provider
/// failure into many; this bounds that cost and matches the eight-entry
/// default for a slow or rate-limited gateway. Each delay is also held to
/// 30 s by the provider layer, so worst-case backoff sleeping is this count
/// times 30 s.
const MAX_RETRY_DELAYS: usize = 8;

/// Default independent agent attempts per question. `1` is the single-run
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

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub profile_name: Option<String>,
    pub profile: Option<DatabaseProfile>,
    pub ai: ResolvedAi,
    /// Trusted opt-in coding admission settings. They remain separate from
    /// authority: enabled configuration never grants a capability by itself.
    pub coding: crate::ResolvedCoding,
    /// Trusted `[agents]` settings: the two bounds a job's helper scheduler runs
    /// under, and the helper type registry its `helpers` tool advertises. Like
    /// the coding settings they shape admission only — a type is a shape, never
    /// a grant.
    pub agents: crate::ResolvedAgents,
    /// The one `[tools]` allow list every agent surface narrows through. It
    /// only ever removes tools: `allow: None` is the default and leaves each
    /// surface exactly as it composes itself.
    pub tools: crate::ResolvedTools,
    /// The validated `[hooks]` table: the user's own commands at the agent's
    /// lifecycle points, every bound already applied. Protected whole, so these
    /// always reflect the trusted layers.
    pub hooks: crate::ResolvedHooks,
    /// The `[web_search]` backend, or none. Saya runs no search service of
    /// its own, so an unconfigured backend means the coding surfaces
    /// advertise no `web_search` at all.
    pub web_search: crate::ResolvedWebSearch,
    /// The `[mcp_servers]` table, validated: the MCP servers a surface may
    /// start and ask for tools. Empty — the default — means no server exists
    /// and no surface advertises an `mcp__` tool at all.
    pub mcp_servers: crate::ResolvedMcpServers,
    /// The `[permissions]` table: whether a session may enter `bypass`.
    /// Protected whole, so this always reflects the trusted layers — a
    /// project layer's attempt to allow the mode is reverted and reported in
    /// [`Self::ignored_project_overrides`].
    pub permissions: crate::ResolvedPermissions,
    /// Every surface's validated run ceilings, resolved from `[limits]`. These
    /// are a configuration contract; runtime enforcement is composed separately
    /// by each surface's own engine.
    pub limits: crate::ResolvedLimits,
    pub max_rows: usize,
    pub read_only: bool,
    /// Independent agent attempts per question. Defaults to `1` (the
    /// single-run behaviour); each extra candidate is a full additional agent
    /// run. Bounded to `1..=16` at resolve time. Read by the `ask` command,
    /// which runs one attempt per candidate and votes on their nominated SQL
    /// (`crates/saya-cli/src/agent/candidates`).
    pub candidates: usize,
    /// The `[jobs]` section resolved: the download budgets a run's
    /// `http_download` calls spend from. The run's own ceilings live in
    /// [`Self::limits`]' `run` sub-table; there is deliberately no environment
    /// input to either.
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
    /// Total deadline for one model request, including a stream that keeps
    /// delivering chunks.
    pub request_timeout_seconds: u64,
    /// Per-response output-token ceiling requested from the provider, or
    /// `None` when no layer stated one — the model's own default then applies.
    pub max_output_tokens: Option<u32>,
    /// How hard the model is asked to think, or `None` when no layer stated
    /// one — the default, which sends no effort field and leaves the
    /// endpoint's own configuration in charge. Distinct from a value: absent
    /// is not `low`, and a provider that cannot spell effort is not an error
    /// because every provider saya has can.
    pub reasoning_effort: Option<crate::ReasoningEffort>,
    /// Whether an `openrouter.ai` request carries OpenRouter's two
    /// app-attribution headers. On unless a layer opted out; no other host
    /// ever receives them.
    pub app_attribution: bool,
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
    /// Share of a known context window, in percent, at which the agent loop
    /// compacts its conversation. Half the window by default.
    pub compaction_trigger_percent: u8,
}

#[path = "resolve/resolution.rs"]
mod resolution;
#[path = "resolve/validation.rs"]
mod validation;

pub use resolution::resolve;
use validation::{
    require_candidates, require_compaction_trigger_percent, require_context_byte_budget,
    require_context_window_tokens, require_retry_delays,
};
