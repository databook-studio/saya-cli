//! Usage counted across a run: the sink's totals over the provider calls'
//! `TokenUsage` reports. The honesty rule is the run contracts' own: an
//! unreported figure means "unknown", never zero.

use saya_agent::TokenUsage;

/// Usage accumulated across a run's provider calls. The two figures every
/// provider reports sum directly; the optional figures — absent meaning "not
/// reported" — sum only over the calls that reported them. A figure no call
/// reported stays `None`, never `Some(0)`, and a reported zero stays a
/// reported zero: a provider that reports nothing must not be read as having
/// cost nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTotals {
    /// Input tokens summed over every call that reported usage.
    pub input_tokens: u64,
    /// Output tokens summed over every call that reported usage.
    pub output_tokens: u64,
    /// Cache reads, summed over the calls that reported them; `None` when no
    /// call reported the figure.
    pub cached_input_tokens: Option<u64>,
    /// Cache writes, summed over the calls that reported them; `None` when
    /// no call reported the figure.
    pub cache_creation_input_tokens: Option<u64>,
    /// Reasoning tokens, summed over the calls that reported them; `None`
    /// when no call reported the figure.
    pub reasoning_tokens: Option<u64>,
}

impl UsageTotals {
    /// Folds one call's reported usage into the totals. An optional figure a
    /// call did not report folds in as nothing: it can neither turn a `None`
    /// total into `Some(0)` nor turn a reported zero into "unknown".
    pub fn fold(&mut self, usage: &TokenUsage) {
        let reported = |total: Option<u64>, seen: Option<u64>| match (total, seen) {
            (Some(total), Some(seen)) => Some(total.saturating_add(seen)),
            (None, seen) => seen,
            (total, None) => total,
        };
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.cached_input_tokens = reported(self.cached_input_tokens, usage.cached_input_tokens);
        self.cache_creation_input_tokens = reported(
            self.cache_creation_input_tokens,
            usage.cache_creation_input_tokens,
        );
        self.reasoning_tokens = reported(self.reasoning_tokens, usage.reasoning_tokens);
    }
}
