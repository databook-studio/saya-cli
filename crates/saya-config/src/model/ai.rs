use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiFile {
    pub provider: Option<AiProvider>,
    /// Provider model name. Endpoint resolution bounds this value, including
    /// when an endpoint inherits it from `[ai]`.
    pub model: Option<String>,
    /// Provider base URL. Endpoint resolution bounds this value, including
    /// when an endpoint inherits it from `[ai]`.
    pub base_url: Option<String>,
    pub allow_data_sharing: Option<bool>,
    pub api_key: Option<SecretRef>,
    pub temperature: Option<f32>,
    /// Total budget for establishing a request or a non-streaming response.
    pub timeout_seconds: Option<u64>,
    /// Maximum silence between stream chunks before the provider is stalled.
    pub idle_timeout_seconds: Option<u64>,
    /// Per-response output-token ceiling requested from the provider.
    pub max_output_tokens: Option<u32>,
    /// Provider retry backoff in milliseconds, tried in order before the
    /// provider gives up. Absent keeps the default three-entry schedule. An
    /// empty list means "do not retry" (one attempt, no sleeps). The list
    /// length is bounded at resolve time.
    pub retry_delays_ms: Option<Vec<u64>>,
    /// Ceiling on the approximate byte size of the conversation the agent loop
    /// assembles and sends to the provider. The loop trims under it (oldest
    /// tool results dropped, newest truncated with a marker) rather than abort.
    pub context_byte_budget: Option<usize>,
    /// Starting policy for an interactive investigation. Raw TOML values stay
    /// intact until resolution so malformed values cannot become unlimited.
    pub investigation_max_logical_answering_requests: Option<toml::Value>,
    pub investigation_max_requested_tool_calls: Option<toml::Value>,
    pub investigation_max_elapsed_seconds: Option<toml::Value>,
    pub investigation_max_known_reported_tokens: Option<toml::Value>,
    /// The model's context window in tokens, as the user declares it. This is
    /// how a model the built-in table does not know — a private gateway serving
    /// a name of its own — gets a window at all. `None` defers to the table.
    pub context_window_tokens: Option<u64>,
    /// Show the model's chain-of-thought in the transcript. Off by default:
    /// thinking is verbose (measured at ~2x the answer length) and restates
    /// database contents in prose, so a user who did not ask for it must not
    /// get it. Display only — reasoning is never persisted regardless of this
    /// setting.
    pub show_thinking: Option<bool>,
    /// How context compaction behaves once the window fills: `auto`
    /// (summarise on crossing the compact threshold), `manual` (only an
    /// explicit `/compact`), or `off` (no automatic trigger and no context
    /// warning). Absent keeps `auto`. An ordinary setting like `show_thinking`:
    /// it shapes local working memory, never where traffic goes.
    pub compaction: Option<crate::CompactionMode>,
    /// Named endpoints a run's roles can bind to (`[[ai.endpoints]]`). Empty
    /// by default: an absent section changes nothing for anyone, and every
    /// role keeps the plain `[ai]` block through the `orchestrator` fallback
    /// at resolution.
    #[serde(default)]
    pub endpoints: Vec<EndpointFile>,
}

/// One `[[ai.endpoints]]` entry: a named endpoint a run's role can bind to.
///
/// The name is required — it is the key roles bind to. Unset fields inherit
/// the plain `[ai]` block at resolution, so an endpoint only overrides what
/// it declares (typically `base_url` and `api_key`); `name` never inherits.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointFile {
    /// The run-scoped endpoint name. The same shape the run contracts use,
    /// validated at resolution against `saya_types::is_name_shaped`.
    pub name: String,
    pub provider: Option<AiProvider>,
    /// Provider model override; bounded at endpoint resolution.
    pub model: Option<String>,
    /// Provider base URL override; bounded at endpoint resolution.
    pub base_url: Option<String>,
    /// A reference (`{ env = ... }`), never an inline value — `SecretRef` is
    /// an untagged enum, so an inline string fails to parse with the
    /// `inline_secret_hint` diagnostic naming this endpoint.
    pub api_key: Option<SecretRef>,
}
