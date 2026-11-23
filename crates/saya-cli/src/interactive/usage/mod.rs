//! Session-wide token usage, and the per-call totals it folds.
//!
//! Both surfaces read [`SessionUsage`]: the line front folds a turn's answering
//! and extraction totals into it, and `/usage` renders it. It lived on the old
//! TUI's `App` until X1 deleted that app; the accumulator is the session's, so
//! it lives beside the session now.

mod totals;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub(crate) use totals::UsageTotals;

use saya_agent::TokenUsage;

/// A reported cost, as money: dollars with up to four decimals and the trailing
/// zeros a reader does not need. `$0.03` for three cents, `$0.0125` for a tenth
/// of a cent, `$0` for nothing. One function, because the status row's meter and
/// `/status`'s cost line are the same figure stated twice.
pub(crate) fn cost_text(micros: u64) -> String {
    let whole = micros / 1_000_000;
    let fraction = micros % 1_000_000;
    if fraction == 0 {
        return format!("${whole}");
    }
    let digits = format!("{fraction:06}");
    format!("${whole}.{}", digits.trim_end_matches('0'))
}

/// Session-wide token usage accumulator. Sums every field `TokenUsage`
/// accounts for, kept in two labelled totals: the answering call and the
/// post-turn extraction (learning) call. The `Option` fields are
/// tracked with a "was this ever reported?" flag so a cache hit rate over
/// unreported data renders as **unknown**, never 0% — absent is not zero.
///
/// In-memory only: the `SessionState` field carrying this is `#[serde(skip)]`,
/// so it never enters a persisted session file. `/clear` resets it, matching
/// the conversation reset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SessionUsage {
    pub(crate) answering: UsageTotals,
    pub(crate) learning: UsageTotals,
    /// Whether any extraction call reported usage. A provider that reported
    /// nothing (`None`) leaves this false so the learning section is omitted
    /// entirely — absent is not zero, and the section would otherwise show a
    /// misleading row of zeros.
    pub(crate) learning_reported: bool,
}

impl SessionUsage {
    /// Folds one answering turn's usage into the answering total. A silent
    /// provider produces an all-zero `TokenUsage`, which the accumulator skips
    /// so a usage-less turn adds nothing.
    pub(crate) fn record(&mut self, usage: &TokenUsage) {
        self.answering.record(usage);
    }

    /// Input tokens the last answering call reported: what the last request
    /// sent, which is what the context meter measures against the window.
    pub(crate) fn last_input_tokens(&self) -> Option<u64> {
        self.answering.last_input_tokens
    }

    /// What the provider reported this session has cost so far, in micro-USD.
    /// `None` until a call reports a cost — a reported zero is `Some(0)`.
    pub(crate) fn cost_micros(&self) -> Option<u64> {
        self.answering.cost_micros
    }

    /// States what the turn's last answering request sent and what the provider
    /// priced it at. Two figures of a call, never the session's sums: the meter
    /// divides one request by the window it was sent to, and the spend adds up
    /// only across costs a provider actually reported.
    pub(crate) fn note_request(&mut self, input_tokens: Option<u64>, cost_micros: Option<u64>) {
        if input_tokens.is_some() {
            self.answering.last_input_tokens = input_tokens;
        }
        if let Some(cost) = cost_micros {
            let spent = self.answering.cost_micros.unwrap_or(0);
            self.answering.cost_micros = Some(spent.saturating_add(cost));
        }
    }
    /// Folds one extraction call's usage into the learning total. `None` (the
    /// provider reported nothing) skips entirely — absent is not zero, and
    /// must stay distinguishable from a reported zero, which is recorded as a
    /// counted call with zero tokens.
    pub(crate) fn record_learning(&mut self, usage: Option<TokenUsage>) {
        if let Some(usage) = usage {
            self.learning_reported = true;
            self.learning.record_call(&usage);
        }
    }

    /// Renders the session usage breakdown for `/usage`. The answering section
    /// is shown whenever there were answering turns; the learning section
    /// appears only when an extraction call reported usage, so a session with
    /// learning disabled renders exactly what it did before the learning total
    /// existed. Each section states the hit-rate formula so a rate over a
    /// merged denominator is never implied.
    pub(crate) fn render(&self) -> String {
        let answering_empty = self.answering.turns == 0;
        let learning_empty = !self.learning_reported;
        if answering_empty && learning_empty {
            return "No token usage reported yet this session.".into();
        }
        let mut out = String::new();
        if !answering_empty {
            out.push_str(&self.answering.render_section("Session token usage"));
        }
        if !learning_empty {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&self.learning.render_section("Learning call"));
        }
        out
    }
}
