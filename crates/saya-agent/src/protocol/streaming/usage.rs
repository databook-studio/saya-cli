use serde::{Deserialize, Serialize};

/// Token counts reported by a provider for one response. Providers that do
/// not report usage simply never emit it.
/// `input_tokens` and `output_tokens` hold complete provider-reported totals;
/// optional cache and reasoning fields preserve provider detail separately.
///
/// Every field beyond `input_tokens`/`output_tokens` is `Option<u64>`: a
/// provider that does not report a number leaves it `None` (invariant: absent
/// is not zero — a cache hit rate computed over `None` is *unknown*, not 0%).
/// The two existing fields keep their meaning and type (`u64`) so callers
/// that sum or copy them are unaffected. The struct stays `Copy` because every
/// field is `Copy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider's prompt cache. This is a
    /// provider-reported breakdown of `input_tokens`, not an additional count.
    /// Wire sources:
    /// OpenAI `usage.prompt_tokens_details.cached_tokens`,
    /// Anthropic `usage.cache_read_input_tokens`,
    /// Gemini `usageMetadata.cachedContentTokenCount`, and Ollama
    /// `prompt_eval_cached_count`. The Ollama value is a subset of
    /// `prompt_eval_count`; `None` when the relevant provider omits this detail.
    pub cached_input_tokens: Option<u64>,
    /// Provider-reported input tokens written to a prompt cache. This remains
    /// an optional breakdown of `input_tokens`, separate from
    /// `cached_input_tokens`, and is never added to the scalar total. Wire
    /// sources are Anthropic `usage.cache_creation_input_tokens` and OpenAI
    /// Chat `usage.prompt_tokens_details.cache_write_tokens` when reported;
    /// `None` means this detail was absent or unsupported.
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens spent on chain-of-thought reasoning. The inclusive/separate
    /// split is per-provider and documented here because a normalising layer
    /// that guesses wrong is worse than the raw number: on **OpenAI** this is
    /// `usage.completion_tokens_details.reasoning_tokens`, which sits *inside*
    /// `completion_tokens` and is therefore **inclusive of `output_tokens`**
    /// — do not add it to `output_tokens`. On **Gemini** it is
    /// `usageMetadata.thoughtsTokenCount`, reported *separately* from
    /// `candidatesTokenCount` and added by the Gemini adapter to form
    /// `output_tokens`. Anthropic counts reasoning inside its output tokens
    /// (no separate field) and Ollama reports none; both leave this `None`.
    pub reasoning_tokens: Option<u64>,
}

impl TokenUsage {
    /// The two counts every provider reports. The optional cache and reasoning
    /// counts stay absent — absent meaning "not reported", which is not the
    /// same as a reported zero — until a provider fills them in. This is the
    /// way in from another crate, where the struct is `#[non_exhaustive]` and a
    /// literal will not compile.
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            ..Self::default()
        }
    }

    /// The same counts with a cache-read figure attached. Takes an `Option` so
    /// a caller passing `None` states "the provider did not report this",
    /// which is not the same as `Some(0)`, a cold cache.
    #[must_use]
    pub fn with_cached_input(mut self, cached_input_tokens: Option<u64>) -> Self {
        self.cached_input_tokens = cached_input_tokens;
        self
    }

    /// The same counts with a cache-write figure attached; `None` means
    /// unreported, as above.
    #[must_use]
    pub fn with_cache_creation(mut self, cache_creation_input_tokens: Option<u64>) -> Self {
        self.cache_creation_input_tokens = cache_creation_input_tokens;
        self
    }

    /// The same counts with a reasoning figure attached; `None` means
    /// unreported, as above.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning_tokens: Option<u64>) -> Self {
        self.reasoning_tokens = reasoning_tokens;
        self
    }
}
