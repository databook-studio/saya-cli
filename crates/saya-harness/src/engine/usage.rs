//! Usage counted across a run: the sink's totals over the provider calls'
//! `TokenUsage` reports, plus the spend the run's journal already records —
//! folded back in on a resume so the totals measure the run's whole life,
//! not one invocation. The honesty rule is the run contracts' own: an
//! unreported figure means "unknown", never zero.

use saya_agent::TokenUsage;
use saya_types::RunEvent;

/// Usage accumulated across a run's provider calls. The two figures every
/// provider reports sum directly; the optional figures — absent meaning "not
/// reported" — sum only over the calls that reported them. A figure no call
/// reported stays `None`, never `Some(0)`, and a reported zero stays a
/// reported zero: a provider that reports nothing must not be read as having
/// cost nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTotals {
    /// Tokens already spent when this accounting took over, carried in from
    /// the run journal on a resume — in the journal's own arithmetic, each
    /// recorded call's input plus output, which is the token ceiling's. The
    /// journal does not carry those calls' input/output split, so the figure
    /// is never folded into the two fields below: splitting it would
    /// fabricate a split the record never held. Zero on a fresh run's sink.
    pub carried_tokens: u64,
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
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.cached_input_tokens = reported(self.cached_input_tokens, usage.cached_input_tokens);
        self.cache_creation_input_tokens = reported(
            self.cache_creation_input_tokens,
            usage.cache_creation_input_tokens,
        );
        self.reasoning_tokens = reported(self.reasoning_tokens, usage.reasoning_tokens);
    }

    /// Folds one journaled [`RunEvent::Usage`] back in — the resume path that
    /// seeds a sink's totals from the run's durable record.
    ///
    /// The journal records the ceiling's own arithmetic: each call's `tokens`
    /// figure is that call's input plus output, and the split between them is
    /// not in the record. The figure therefore folds into
    /// [`UsageTotals::carried_tokens`] — the spend as the record holds it —
    /// and never into `input_tokens`/`output_tokens`, which stay this
    /// invocation's own reports. An event whose `tokens` is `None` said
    /// nothing about the call's cost and adds nothing to the sum, not a zero.
    /// The optional figures fold by the same rule [`UsageTotals::fold`] uses:
    /// a figure the record carries sums in, and one it does not leaves the
    /// total as it stands — unknown stays unknown, never zero. The journal
    /// records no reasoning tokens, so this path cannot restore them; they
    /// stay unknown until this invocation's own calls report.
    pub fn fold_journaled(&mut self, event: &RunEvent) {
        let RunEvent::Usage {
            tokens,
            cached_input_tokens,
            cache_creation_input_tokens,
            ..
        } = event
        else {
            return;
        };
        if let Some(tokens) = tokens {
            self.carried_tokens = self.carried_tokens.saturating_add(*tokens);
        }
        self.cached_input_tokens = reported(self.cached_input_tokens, *cached_input_tokens);
        self.cache_creation_input_tokens = reported(
            self.cache_creation_input_tokens,
            *cache_creation_input_tokens,
        );
    }

    /// The totals a run's sink starts from on a resume: the journal's usage
    /// events folded back in. Callers must pass the repaired record — the
    /// events read after `truncate_torn_tail` dropped the torn tail — so a
    /// half-written usage line never counts toward the carried spend.
    pub fn from_journal(events: &[RunEvent]) -> Self {
        let mut totals = Self::default();
        for event in events {
            totals.fold_journaled(event);
        }
        totals
    }
}

/// Sums one optional figure the honesty way: reported figures sum, an
/// unreported one leaves the total as it stands — `None` stays `None`, a
/// reported zero stays a reported zero.
fn reported(total: Option<u64>, seen: Option<u64>) -> Option<u64> {
    match (total, seen) {
        (Some(total), Some(seen)) => Some(total.saturating_add(seen)),
        (None, seen) => seen,
        (total, None) => total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seeding path's honesty rule, the resume-side twin of the sink's
    /// own accumulation test: figures the journal does not carry stay
    /// unknown, never zero, and a recorded zero stays a recorded zero.
    #[test]
    fn seeding_keeps_the_journals_unreported_figures_unknown() {
        let usage =
            |tokens: Option<u64>, cached: Option<u64>, cache_creation: Option<u64>| -> RunEvent {
                RunEvent::Usage {
                    endpoint: "orchestrator".into(),
                    tokens,
                    turns: None,
                    tool_calls: None,
                    cached_input_tokens: cached,
                    cache_creation_input_tokens: cache_creation,
                }
            };

        // Two calls that reported only their combined cost: the carried
        // spend sums them, and every optional figure stays unknown.
        let seeded =
            UsageTotals::from_journal(&[usage(Some(100), None, None), usage(Some(60), None, None)]);
        assert_eq!(seeded.carried_tokens, 160, "recorded figures sum");
        assert_eq!(
            seeded.cached_input_tokens, None,
            "a figure no recorded call reported stays unknown, never zero"
        );
        assert_eq!(seeded.cache_creation_input_tokens, None);
        assert_eq!(seeded.reasoning_tokens, None, "the journal carries none");
        assert_eq!(
            (seeded.input_tokens, seeded.output_tokens),
            (0, 0),
            "the record holds no split, and none is fabricated"
        );

        // A recorded zero is a number: it seeds `Some(0)`, and a later
        // recorded figure sums onto it.
        let seeded = UsageTotals::from_journal(&[
            usage(Some(40), Some(0), None),
            usage(Some(20), None, Some(2)),
        ]);
        assert_eq!(seeded.carried_tokens, 60);
        assert_eq!(
            seeded.cached_input_tokens,
            Some(0),
            "a recorded zero is a number, not 'not reported'"
        );
        assert_eq!(
            seeded.cache_creation_input_tokens,
            Some(2),
            "the last event did not report it; it must not fold in as zero"
        );

        // An event whose cost is unknown adds nothing to the sum — not a
        // zero — while whatever it did report still folds in.
        let seeded = UsageTotals::from_journal(&[usage(None, Some(7), None)]);
        assert_eq!(seeded.carried_tokens, 0, "unknown cost is not zero");
        assert_eq!(seeded.cached_input_tokens, Some(7));
    }

    /// Seeding never invents a transition out of a non-usage event: the
    /// replay's lifecycle events fold in as nothing.
    #[test]
    fn seeding_folds_usage_events_only() {
        let seeded = UsageTotals::from_journal(&[
            RunEvent::RunStarted,
            RunEvent::PlanApproved,
            RunEvent::Paused {
                reason: saya_types::PauseReason::BudgetExhausted,
            },
            RunEvent::Usage {
                endpoint: "orchestrator".into(),
                tokens: Some(5),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        ]);
        assert_eq!(
            seeded,
            UsageTotals {
                carried_tokens: 5,
                ..UsageTotals::default()
            }
        );
        // A run that never reported usage seeds the same totals a fresh
        // sink holds: the resume behaves exactly as it did before seeding.
        assert_eq!(
            UsageTotals::from_journal(&[RunEvent::RunStarted, RunEvent::PlanApproved]),
            UsageTotals::default()
        );
    }
}
