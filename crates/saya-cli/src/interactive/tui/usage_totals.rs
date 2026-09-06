//! Per-call token-usage accumulator shared by the answering and learning totals.
//!
//! Both the answering call and the post-turn extraction call spend tokens the
//! user paid for. This type folds one call kind's usage across turns, keeping
//! the "was this ever reported?" flags that let `/usage` distinguish a reported
//! zero from an absent field — absent is not zero.

use saya_agent::TokenUsage;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UsageTotals {
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) reasoning_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) cache_creation_input_tokens: u64,
    pub(crate) turns: u32,
    pub(crate) reported_cached: bool,
    pub(crate) reported_cache_creation: bool,
    pub(crate) reported_reasoning: bool,
}

impl UsageTotals {
    /// Folds one answering turn's usage, skipping a silent (all-zero) report.
    /// A silent provider produces all-zero counters; counting that as a turn
    /// would inflate the turn count without adding tokens.
    pub(crate) fn record(&mut self, usage: &TokenUsage) {
        if usage.input_tokens == 0 && usage.output_tokens == 0 {
            return;
        }
        self.accumulate(usage);
        self.turns += 1;
    }

    /// Folds one extraction call's usage, counting the call even when the
    /// provider reported zeros. A reported zero is a real "this cost nothing"
    /// and must stay distinguishable from a provider that reported nothing at
    /// all, so the call is always counted rather than skipped.
    pub(crate) fn record_call(&mut self, usage: &TokenUsage) {
        self.accumulate(usage);
        self.turns += 1;
    }

    fn accumulate(&mut self, usage: &TokenUsage) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        if let Some(cached) = usage.cached_input_tokens {
            self.cached_input_tokens += cached;
            self.reported_cached = true;
        }
        if let Some(created) = usage.cache_creation_input_tokens {
            self.cache_creation_input_tokens += created;
            self.reported_cache_creation = true;
        }
        if let Some(reasoning) = usage.reasoning_tokens {
            self.reasoning_tokens += reasoning;
            self.reported_reasoning = true;
        }
    }

    /// The cache hit rate as a percentage string, or "unknown" when no turn
    /// reported cached tokens or there was no input. The formula is
    /// `Σcached / Σinput` — the honest ratio of sums across all turns, not a
    /// mean of per-turn rates. Turns that did not report cache tokens
    /// contribute their input to the denominator but 0 to the numerator, so
    /// the rate is a lower bound, not an invention.
    fn cache_hit_rate(&self) -> String {
        if !self.reported_cached || self.input_tokens == 0 {
            return "unknown".into();
        }
        let rate = (self.cached_input_tokens as f64 / self.input_tokens as f64) * 100.0;
        format!("{rate:.0}%")
    }

    /// Renders one labelled section: a header line naming the call kind, then
    /// the indented breakdown. Each `Option` field shows its total or `—` when
    /// no turn reported it; the hit rate states the formula so a reader knows
    /// what the number is.
    pub(crate) fn render_section(&self, label: &str) -> String {
        let dash = "—";
        let opt = |reported: bool, value: u64| -> String {
            if reported {
                value.to_string()
            } else {
                dash.into()
            }
        };
        let rate = self.cache_hit_rate();
        let formula = if self.reported_cached {
            "Σcached / Σinput"
        } else {
            "Σcached / Σinput; no provider reported cache data"
        };
        format!(
            "{label} ({turns} turn{plural}):\n\n\
             \x20 Input tokens: {input}\n\
             \x20 Output tokens: {output}\n\
             \x20 Reasoning tokens: {reasoning}\n\
             \x20 Cached input: {cached}\n\
             \x20 Cache creation: {cache_creation}\n\
             \x20 Cache hit rate: {rate} ({formula})",
            label = label,
            turns = self.turns,
            plural = if self.turns == 1 { "" } else { "s" },
            input = self.input_tokens,
            output = self.output_tokens,
            reasoning = opt(self.reported_reasoning, self.reasoning_tokens),
            cached = opt(self.reported_cached, self.cached_input_tokens),
            cache_creation = opt(
                self.reported_cache_creation,
                self.cache_creation_input_tokens
            ),
            rate = rate,
            formula = formula,
        )
    }
}
