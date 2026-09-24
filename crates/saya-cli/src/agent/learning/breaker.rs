//! The per-session extraction circuit breaker.
//!
//! Post-turn extraction has no wall-clock ceiling (see
//! [`super::post_turn::run_post_turn_extraction`]) — an explicit owner
//! decision that departs from AGENTS.md's "bound untrusted work — time" rule
//! for this one path. In its place, two consecutive **misses** — a reply cut
//! off at the output limit ([`ProviderError::OutputTruncated`]) or the
//! transport stalling or timing out — disable further extraction requests
//! for the rest of the session: no gate evaluation, no
//! `KnowledgeLearningStarted`, no provider call. Every other outcome —
//! success, a parse failure, a non-JSON reply, an HTTP error, an ingest
//! failure — resets the count to 0.
//!
//! Not persisted: a new or resumed session always starts enabled. With no
//! session (`saya ask`, a candidate attempt), the caller uses a fresh
//! breaker per call, which can never trip within one turn — correct, since
//! there is no "next turn" for it to disable.

use saya_agent::ProviderError;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::ExtractionRunnerError;

/// One extraction attempt's outcome, from the breaker's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptOutcome {
    Miss,
    Other,
}

/// Classifies a failed extraction the way the breaker counts it.
///
/// The transport surfaces a stall, an idle timeout, and a request timeout
/// all as [`ProviderError::Request`] — there is no dedicated variant — so
/// the miss is recognised by the message the transport wrote: `"provider
/// stream stalled"` (the per-chunk idle budget in
/// `providers/{openai,anthropic,ollama}_stream.rs`), `"…timed out while
/// establishing a connection…"` (the connect timeout shared by every
/// provider in `providers/http.rs::send_stream`), and `"…timed out while
/// reading the response…"` (Gemini's own total-read timeout, which does not
/// stream chunk-by-chunk). Every other `Request` message — an HTTP status
/// line, "could not reach the provider", a stream byte-limit refusal — is
/// not a miss.
pub(crate) fn classify(error: &ExtractionRunnerError) -> AttemptOutcome {
    match error {
        ExtractionRunnerError::Provider(provider_error) if is_transport_miss(provider_error) => {
            AttemptOutcome::Miss
        }
        _ => AttemptOutcome::Other,
    }
}

fn is_transport_miss(error: &ProviderError) -> bool {
    match error {
        ProviderError::OutputTruncated { .. } => true,
        ProviderError::Request(message) => {
            message.contains("stalled") || message.contains("timed out")
        }
        _ => false,
    }
}

/// A session's extraction circuit breaker.
#[derive(Debug, Default)]
pub(crate) struct LearningBreaker {
    misses: AtomicU32,
    disabled: AtomicBool,
}

impl LearningBreaker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether the breaker has already tripped: the caller must make no
    /// extraction request at all when this is true.
    pub(crate) fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::Relaxed)
    }

    /// Records one attempt's outcome. Returns the miss count the instant it
    /// reaches two — the caller's cue to emit `KnowledgeLearningDisabled`
    /// exactly once — and `None` on every other call.
    pub(crate) fn record(&self, outcome: AttemptOutcome) -> Option<u32> {
        match outcome {
            AttemptOutcome::Other => {
                self.misses.store(0, Ordering::Relaxed);
                None
            }
            AttemptOutcome::Miss => {
                let misses = self.misses.fetch_add(1, Ordering::Relaxed) + 1;
                if misses == 2 {
                    self.disabled.store(true, Ordering::Relaxed);
                    Some(misses)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_consecutive_misses_trip_the_breaker_once() {
        let breaker = LearningBreaker::new();
        assert!(!breaker.is_disabled());
        assert_eq!(breaker.record(AttemptOutcome::Miss), None);
        assert!(!breaker.is_disabled(), "one miss must not trip it");
        assert_eq!(breaker.record(AttemptOutcome::Miss), Some(2));
        assert!(breaker.is_disabled());
    }

    #[test]
    fn a_success_between_misses_resets_the_count() {
        let breaker = LearningBreaker::new();
        assert_eq!(breaker.record(AttemptOutcome::Miss), None);
        assert_eq!(breaker.record(AttemptOutcome::Other), None);
        assert_eq!(breaker.record(AttemptOutcome::Miss), None, "count reset");
        assert!(!breaker.is_disabled());
    }

    #[test]
    fn classify_treats_output_truncated_as_a_miss() {
        let error = ExtractionRunnerError::Provider(ProviderError::output_truncated(
            String::new(),
            Vec::new(),
        ));
        assert_eq!(classify(&error), AttemptOutcome::Miss);
    }

    #[test]
    fn classify_treats_a_stalled_stream_as_a_miss() {
        let error = ExtractionRunnerError::Provider(ProviderError::Request(
            "provider stream stalled".into(),
        ));
        assert_eq!(classify(&error), AttemptOutcome::Miss);
    }

    #[test]
    fn classify_treats_a_connect_timeout_and_a_read_timeout_as_misses() {
        let connect = ExtractionRunnerError::Provider(ProviderError::Request(
            "provider request timed out while establishing a connection to http://x".into(),
        ));
        let read = ExtractionRunnerError::Provider(ProviderError::Request(
            "provider request timed out while reading the response from http://x".into(),
        ));
        assert_eq!(classify(&connect), AttemptOutcome::Miss);
        assert_eq!(classify(&read), AttemptOutcome::Miss);
    }

    #[test]
    fn classify_treats_an_http_error_and_unreachable_provider_as_non_misses() {
        let http_error = ExtractionRunnerError::Provider(ProviderError::Request(
            "HTTP 401: authentication failed — check the API key configured for this provider"
                .into(),
        ));
        let unreachable = ExtractionRunnerError::Provider(ProviderError::Request(
            "could not reach the provider at http://x — check that it is running and the configured base_url is correct".into(),
        ));
        assert_eq!(classify(&http_error), AttemptOutcome::Other);
        assert_eq!(classify(&unreachable), AttemptOutcome::Other);
    }
}
