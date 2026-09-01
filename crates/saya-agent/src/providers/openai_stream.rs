use super::{framing::whitespace, openai_chunks::Chunk, tool_assembly::ToolAssembly};
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
        // Per-chunk idle budget instead of a total cap (see anthropic_stream).
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
struct State {
    bytes: Vec<u8>,
    pending: VecDeque<ProviderEvent>,
    tools: ToolAssembly,
    usage: TokenUsage,
    content: bool,
    done: bool,
}
impl State {
    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        if self.bytes.len().saturating_add(chunk.len()) > crate::MAX_STREAM_BYTES {
            return Err(ProviderError::Request(
                "provider stream exceeded size limit".into(),
            ));
        }
        self.bytes.extend_from_slice(chunk);
        while let Some((end, skip)) = boundary(&self.bytes) {
            let frame = String::from_utf8(self.bytes[..end].to_vec())
                .map_err(|_| ProviderError::InvalidResponse)?;
            self.bytes.drain(..end + skip);
            let data = frame
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                self.done = true;
                break;
            }
            let chunk: Chunk =
                serde_json::from_str(&data).map_err(|_| ProviderError::InvalidResponse)?;
            if let Some(usage) = chunk.usage {
                apply_usage(&mut self.usage, &usage);
                self.pending.push_back(ProviderEvent::Usage(self.usage));
            }
            let Some(choice) = chunk.choices.into_iter().next() else {
                // The trailing usage-only chunk carries an empty choices list.
                if self.usage == TokenUsage::default() {
                    return Err(ProviderError::InvalidResponse);
                }
                continue;
            };
            if let Some(reason) = choice.finish_reason.as_deref()
                && !matches!(reason, "stop" | "tool_calls")
            {
                // `length` is diagnosable (raise the output cap); anything
                // else stays a generic protocol failure.
                if reason == "length" {
                    return Err(ProviderError::Request(
                        "output truncated: the model hit its output-token limit".into(),
                    ));
                }
                return Err(ProviderError::InvalidResponse);
            }
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                self.content = true;
                self.pending.push_back(ProviderEvent::TextDelta(text));
            }
            if let Some(reasoning) = choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                self.pending
                    .push_back(ProviderEvent::ReasoningDelta(reasoning));
            }
            for call in choice.delta.tool_calls {
                self.tools.push(
                    call.index,
                    call.id.as_deref(),
                    call.function.name.as_deref(),
                    call.function.arguments.as_deref(),
                )?;
            }
        }
        if self.done {
            if !whitespace(&self.bytes) || (!self.content && self.tools.is_empty()) {
                return Err(ProviderError::InvalidResponse);
            }
            let calls = std::mem::take(&mut self.tools).finish()?;
            if !calls.is_empty() {
                self.pending.push_back(ProviderEvent::ToolCalls(calls));
            }
            self.bytes.clear();
            self.pending.push_back(ProviderEvent::Done);
        }
        Ok(())
    }
}
fn boundary(value: &[u8]) -> Option<(usize, usize)> {
    value
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| {
            value
                .windows(2)
                .position(|part| part == b"\n\n")
                .map(|i| (i, 2))
        })
}

/// Folds an OpenAI usage chunk into the running totals for one response. The
/// two existing counters are overwritten when present (OpenAI reports them
/// cumulatively on the trailing chunk); the cache/reasoning detail fields are
/// carried through as `Option`, so a reported `0` stays `Some(0)` and an
/// omitted field stays `None`.
fn apply_usage(accumulated: &mut TokenUsage, usage: &super::openai_chunks::Usage) {
    if let Some(input) = usage.prompt_tokens {
        accumulated.input_tokens = input;
    }
    if let Some(output) = usage.completion_tokens {
        accumulated.output_tokens = output;
    }
    accumulated.cached_input_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens);
    accumulated.reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens);
}

#[cfg(test)]
mod tests {
    use super::{TokenUsage, apply_usage};
    use serde_json::json;

    /// Deliverable 4 (OpenAI mapping, with): a usage chunk with detail objects
    /// populates `cached_input_tokens` and `reasoning_tokens` on the running
    /// total.
    #[test]
    fn apply_usage_populates_cache_and_reasoning() {
        let mut accumulated = TokenUsage::default();
        let usage: super::super::openai_chunks::Usage = serde_json::from_value(json!({
            "prompt_tokens": 100,
            "completion_tokens": 200,
            "prompt_tokens_details": {"cached_tokens": 90},
            "completion_tokens_details": {"reasoning_tokens": 270}
        }))
        .expect("parses");
        apply_usage(&mut accumulated, &usage);
        assert_eq!(accumulated.input_tokens, 100);
        assert_eq!(accumulated.output_tokens, 200);
        assert_eq!(accumulated.cached_input_tokens, Some(90));
        assert_eq!(accumulated.reasoning_tokens, Some(270));
        assert_eq!(accumulated.cache_creation_input_tokens, None);
    }

    /// Deliverable 4 (OpenAI mapping, without): a usage chunk with no detail
    /// objects leaves the new fields `None` and does not clobber a previously
    /// reported cache hit — the trailing chunk for one response carries the
    /// final numbers, but detail absence is an honest `None`, not a reset.
    #[test]
    fn apply_usage_without_details_leaves_new_fields_none() {
        let mut accumulated = TokenUsage::default();
        let usage: super::super::openai_chunks::Usage =
            serde_json::from_value(json!({"prompt_tokens": 5, "completion_tokens": 6}))
                .expect("parses");
        apply_usage(&mut accumulated, &usage);
        assert_eq!(accumulated.input_tokens, 5);
        assert_eq!(accumulated.output_tokens, 6);
        assert_eq!(accumulated.cached_input_tokens, None);
        assert_eq!(accumulated.reasoning_tokens, None);
    }

    /// Deliverable 5 (OpenAI mapping): a reported `cached_tokens: 0` flows
    /// through as `Some(0)`, never collapsed to `None`.
    #[test]
    fn apply_usage_reports_zero_cache_as_some_zero() {
        let mut accumulated = TokenUsage::default();
        let usage: super::super::openai_chunks::Usage = serde_json::from_value(json!({
            "prompt_tokens": 10,
            "prompt_tokens_details": {"cached_tokens": 0}
        }))
        .expect("parses");
        apply_usage(&mut accumulated, &usage);
        assert_eq!(accumulated.cached_input_tokens, Some(0));
    }
}
