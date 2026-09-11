//! The per-endpoint usage view `saya run show` renders: the figures a run's
//! journal recorded, folded per endpoint and shaped as one stanza section.
//!
//! The honesty rule is the run contracts' own (U4): a figure no call
//! reported is `None`, never `Some(0)`, and a reported zero stays a reported
//! zero — the fold here works the engine sink's `UsageTotals` fold, over the
//! journal's `RunEvent::Usage` events instead of provider calls. A display
//! that read `cache: 0` for an unreported figure would tell the user the
//! cache missed; `unknown` tells them the provider did not say.

use crate::render_run::count_text;
use saya_types::RunEvent;
use std::collections::BTreeMap;

/// One endpoint's usage as `saya run show` renders it: the figures the run's
/// journal recorded for that endpoint. Every count is `None` while no call
/// reported it — unknown, never zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EndpointUsage {
    pub endpoint: String,
    pub tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub turns: Option<u64>,
    pub tool_calls: Option<u64>,
}

/// The per-endpoint usage a run's journal records, in endpoint order. Each
/// figure is folded the way the engine's `UsageTotals` folds a provider
/// call: reported figures sum over the events that reported them, and a
/// figure no event reported stays `None` — never defaulted to `Some(0)`,
/// never overwritten to unknown by a later report.
pub(crate) fn usage_by_endpoint(events: &[RunEvent]) -> Vec<EndpointUsage> {
    let reported = |total: Option<u64>, seen: Option<u64>| match (total, seen) {
        (Some(total), Some(seen)) => Some(total.saturating_add(seen)),
        (None, seen) => seen,
        (total, None) => total,
    };
    let mut by_endpoint: BTreeMap<&str, EndpointUsage> = BTreeMap::new();
    for event in events {
        if let RunEvent::Usage {
            endpoint,
            tokens,
            turns,
            tool_calls,
            cached_input_tokens,
            cache_creation_input_tokens,
        } = event
        {
            let entry = by_endpoint
                .entry(endpoint.as_str())
                .or_insert_with(|| EndpointUsage {
                    endpoint: endpoint.clone(),
                    tokens: None,
                    cached_input_tokens: None,
                    cache_creation_input_tokens: None,
                    turns: None,
                    tool_calls: None,
                });
            entry.tokens = reported(entry.tokens, *tokens);
            entry.cached_input_tokens = reported(entry.cached_input_tokens, *cached_input_tokens);
            entry.cache_creation_input_tokens = reported(
                entry.cache_creation_input_tokens,
                *cache_creation_input_tokens,
            );
            entry.turns = reported(entry.turns, *turns);
            entry.tool_calls = reported(entry.tool_calls, *tool_calls);
        }
    }
    by_endpoint.into_values().collect()
}

/// One endpoint's usage as the stanza renders it — the same figure shapes
/// and the same `unknown` rule the live wire's event line uses.
pub(crate) fn usage_line(entry: &EndpointUsage) -> String {
    format!(
        "{endpoint} · tokens {tokens} · cache reads {reads} · cache writes {writes} · \
         turns {turns} · tool calls {calls}",
        endpoint = entry.endpoint,
        tokens = count_text(entry.tokens),
        reads = count_text(entry.cached_input_tokens),
        writes = count_text(entry.cache_creation_input_tokens),
        turns = count_text(entry.turns),
        calls = count_text(entry.tool_calls),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_run::{RunShowStanza, run_show_text};

    fn usage_of(events: &[RunEvent]) -> Vec<EndpointUsage> {
        usage_by_endpoint(events)
    }

    /// The one that matters (U4): a cache figure no call reported renders
    /// `unknown`, and a reported zero renders `0` — different answers the
    /// display must not conflate. Both cases ride the chain a real run does:
    /// the journal's usage events folded per endpoint, rendered in the
    /// shared stanza.
    #[test]
    fn an_unreported_cache_figure_renders_unknown_and_a_reported_zero_renders_zero() {
        let never_reported = usage_of(&[
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(100),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(60),
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        ]);
        let reported_zero = usage_of(&[RunEvent::Usage {
            endpoint: "primary".into(),
            tokens: Some(160),
            turns: None,
            tool_calls: None,
            cached_input_tokens: Some(0),
            cache_creation_input_tokens: None,
        }]);
        let unknown_text = run_show_text(stanza_with_usage(&never_reported));
        let zero_text = run_show_text(stanza_with_usage(&reported_zero));
        assert!(
            unknown_text.contains("cache reads unknown"),
            "a provider that never reported cache renders unknown: {unknown_text}"
        );
        assert!(
            zero_text.contains("cache reads 0"),
            "a provider that reported zero renders 0: {zero_text}"
        );
        assert_ne!(
            unknown_text, zero_text,
            "unknown and a reported zero are different answers"
        );
        assert!(
            !unknown_text.contains("cache reads 0") && !zero_text.contains("cache reads unknown"),
            "neither answer may stand in for the other: {unknown_text} | {zero_text}"
        );
    }

    /// Folding sums each figure only over the events that reported it, per
    /// endpoint: a figure one call reported and another did not keeps the
    /// reported sum, a figure no call reported stays unknown, and endpoints
    /// never merge their figures.
    #[test]
    fn usage_folds_per_endpoint_summing_only_the_figures_calls_reported() {
        let usage = usage_of(&[
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(100),
                turns: Some(2),
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "primary".into(),
                tokens: Some(60),
                turns: None,
                tool_calls: Some(3),
                cached_input_tokens: Some(90),
                cache_creation_input_tokens: None,
            },
            RunEvent::Usage {
                endpoint: "other".into(),
                tokens: None,
                turns: None,
                tool_calls: None,
                cached_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        ]);
        assert_eq!(usage.len(), 2, "one entry per endpoint, never merged");
        let primary = &usage[usage
            .iter()
            .position(|entry| entry.endpoint == "primary")
            .unwrap()];
        assert_eq!(primary.endpoint, "primary");
        assert_eq!(primary.tokens, Some(160), "reported figures sum");
        assert_eq!(primary.cached_input_tokens, Some(90));
        assert_eq!(
            primary.cache_creation_input_tokens, None,
            "a figure no call reported stays unknown"
        );
        assert_eq!(primary.turns, Some(2));
        assert_eq!(primary.tool_calls, Some(3));
        let other = &usage[usage
            .iter()
            .position(|entry| entry.endpoint == "other")
            .unwrap()];
        assert_eq!(other.endpoint, "other");
        assert_eq!(
            other.tokens, None,
            "a call that reported nothing is unknown, not zero"
        );
    }

    /// The stanza helper the usage tests build through.
    fn stanza_with_usage(usage: &[EndpointUsage]) -> RunShowStanza<'_> {
        RunShowStanza {
            id: "r-u",
            status: "completed",
            failure_cause: None,
            created_unix_ms: 1,
            updated_unix_ms: 2,
            spec: None,
            paused: None,
            deliverables: &[],
            usage,
        }
    }
}
