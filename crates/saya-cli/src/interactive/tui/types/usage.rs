//! Session-wide token usage accumulator for `/usage`.

use super::super::usage_totals::UsageTotals;
use saya_agent::TokenUsage;

/// Session-wide token usage accumulator. Sums every field the usage-accounting
/// slice widened `TokenUsage` with, kept in two labelled totals: the answering
/// call and the post-turn extraction (learning) call. The `Option` fields are
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
