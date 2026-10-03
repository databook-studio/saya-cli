use crate::{CancellationToken, ProviderError, ProviderEvent, ProviderStream, TokenUsage};
use futures_util::{StreamExt, stream};
use reqwest::Response;
use std::{collections::VecDeque, time::Duration};

pub(super) fn parse(
    response: Response,
    cancellation: CancellationToken,
    idle: Duration,
) -> ProviderStream {
    Box::pin(stream::unfold(
        (
            response.bytes_stream(),
            State::default(),
            cancellation,
            idle,
        ),
        next,
    ))
}

async fn next<S>(
    mut value: (S, State, CancellationToken, Duration),
) -> Option<(
    Result<ProviderEvent, ProviderError>,
    (S, State, CancellationToken, Duration),
)>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    loop {
        if value.2.is_cancelled() {
            value.1.done = true;
            return Some((Err(ProviderError::Cancelled), value));
        }
        if let Some(event) = value.1.pending.pop_front() {
            return Some((Ok(event), value));
        }
        if value.1.done {
            return None;
        }
        // Streams are bounded per chunk gap, not by a total cap: a healthy
        // long generation never times out, a stalled one fails fast.
        let item = tokio::select! {
            _ = value.2.cancelled() => return Some((Err(ProviderError::Cancelled), value)),
            item = tokio::time::timeout(value.3, value.0.next()) => match item {
                Ok(item) => item,
                Err(_) => {
                    value.1.done = true;
                    return Some((
                        Err(ProviderError::Request("provider stream stalled".into())),
                        value,
                    ));
                }
            }
        };
        let Some(chunk) = item else {
            value.1.done = true;
            return Some((Err(ProviderError::InvalidResponse), value));
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                value.1.done = true;
                return Some((
                    Err(ProviderError::Request("network request failed".into())),
                    value,
                ));
            }
        };
        if let Err(error) = value.1.push(&chunk) {
            value.1.done = true;
            return Some((Err(error), value));
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) bytes: Vec<u8>,
    pub(super) frames: super::framing::FrameCount,
    pub(super) pending: VecDeque<ProviderEvent>,
    pub(super) tools: super::tool_assembly::ToolAssembly,
    pub(super) usage: TokenUsage,
    pub(super) raw_input_tokens: Option<u64>,
    pub(super) usage_reported: bool,
    pub(super) done: bool,
    pub(super) assembled_bytes: usize,
    /// Text the wire emitted before any truncation signal, kept so the typed
    /// error can carry the partial answer without re-walking emitted events.
    pub(super) text: String,
    /// The `stop_reason` the final `message_delta` reported, checked at
    /// `message_stop`. A capped response reports `"max_tokens"`.
    pub(super) stop_reason: Option<String>,
}

impl State {
    pub(super) fn reserve(&mut self, bytes: usize) -> Result<(), ProviderError> {
        let next = self
            .assembled_bytes
            .checked_add(bytes)
            .ok_or_else(size_error)?;
        if next
            .checked_add(self.tools.bytes())
            .is_none_or(|total| total > crate::MAX_STREAM_BYTES)
        {
            return Err(size_error());
        }
        self.assembled_bytes = next;
        Ok(())
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        super::anthropic_events::push(self, chunk)
    }
}

pub(super) fn size_error() -> ProviderError {
    ProviderError::Request("provider stream exceeded size limit".into())
}

/// Folds an Anthropic usage snapshot. Its raw input count excludes cache
/// reads and writes, so `TokenUsage` carries their saturating total.
pub(super) fn apply_usage(
    accumulated: &mut TokenUsage,
    raw_input_tokens: &mut Option<u64>,
    usage: &serde_json::Value,
) -> bool {
    let mut changed = false;
    if let Some(input) = usage["input_tokens"].as_u64() {
        *raw_input_tokens = Some(input);
        changed = true;
    }
    if let Some(output) = usage["output_tokens"].as_u64() {
        accumulated.output_tokens = output;
        changed = true;
    }
    if let Some(cached) = usage["cache_read_input_tokens"].as_u64() {
        accumulated.cached_input_tokens = Some(cached);
        changed = true;
    }
    if let Some(created) = usage["cache_creation_input_tokens"].as_u64() {
        accumulated.cache_creation_input_tokens = Some(created);
        changed = true;
    }
    if changed && let Some(input) = raw_input_tokens {
        accumulated.input_tokens = input
            .saturating_add(accumulated.cached_input_tokens.unwrap_or(0))
            .saturating_add(accumulated.cache_creation_input_tokens.unwrap_or(0));
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::{TokenUsage, apply_usage};
    use serde_json::json;

    /// Deliverable 4 (Anthropic, with): a `message_start` usage object
    /// carrying both cache numbers populates the two cache fields (distinct,
    /// because reads and creation bill differently).
    #[test]
    fn message_start_with_cache_fields_populates_both() {
        let mut accumulated = TokenUsage::default();
        let mut raw_input = None;
        let usage = json!({
            "input_tokens": 100,
            "cache_read_input_tokens": 90,
            "cache_creation_input_tokens": 10
        });
        assert!(apply_usage(&mut accumulated, &mut raw_input, &usage));
        assert_eq!(accumulated.input_tokens, 200);
        assert_eq!(accumulated.cached_input_tokens, Some(90));
        assert_eq!(accumulated.cache_creation_input_tokens, Some(10));
        assert_eq!(accumulated.reasoning_tokens, None);
    }

    /// Deliverable 4 (Anthropic, without): a usage object with only the
    /// existing counters leaves the cache fields `None`. The cache numbers
    /// appear only on `message_start`; a `message_delta` carrying only
    /// `output_tokens` must not zero out a previously reported cache read.
    #[test]
    fn usage_without_cache_fields_leaves_them_none() {
        let mut accumulated = TokenUsage {
            input_tokens: 190,
            cached_input_tokens: Some(90),
            ..Default::default()
        };
        let mut raw_input = Some(100);
        let usage = json!({"output_tokens": 34});
        assert!(apply_usage(&mut accumulated, &mut raw_input, &usage));
        assert_eq!(accumulated.output_tokens, 34);
        assert_eq!(accumulated.input_tokens, 190);
        // The earlier cache read survives; `message_delta` did not report it.
        assert_eq!(accumulated.cached_input_tokens, Some(90));
        assert_eq!(accumulated.cache_creation_input_tokens, None);
    }

    /// Deliverable 5 (Anthropic): a reported `cache_read_input_tokens: 0` (a
    /// cache that was created but read nothing) stays `Some(0)`, not `None`.
    #[test]
    fn reported_zero_cache_read_is_some_zero() {
        let mut accumulated = TokenUsage::default();
        let mut raw_input = None;
        let usage = json!({"input_tokens": 10, "cache_read_input_tokens": 0});
        apply_usage(&mut accumulated, &mut raw_input, &usage);
        assert_eq!(accumulated.cached_input_tokens, Some(0));
    }

    /// An empty usage object reports nothing: no `Usage` event should fire, so
    /// the helper returns false and leaves the accumulator untouched.
    #[test]
    fn empty_usage_object_reports_nothing() {
        let mut accumulated = TokenUsage {
            input_tokens: 7,
            ..Default::default()
        };
        let mut raw_input = Some(7);
        assert!(!apply_usage(&mut accumulated, &mut raw_input, &json!({})));
        assert_eq!(accumulated.input_tokens, 7);
    }
}
