use super::{
    framing::{boundary, whitespace},
    openai_chunks::Chunk,
    tool_assembly::ToolAssembly,
};
use crate::{CancellationToken, ProviderError, ProviderEvent, ProviderStream, TokenUsage};
use futures_util::stream;
use reqwest::Response;
use std::{collections::VecDeque, time::Duration};

mod driver;

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
        driver::next,
    ))
}

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    frames: super::framing::FrameCount,
    pending: VecDeque<ProviderEvent>,
    tools: ToolAssembly,
    usage: TokenUsage,
    usage_reported: bool,
    content: bool,
    done: bool,
    /// Truncation error held back until the driver has drained `pending`.
    deferred: Option<ProviderError>,
    /// A length truncation was seen: later frames are read for their usage only.
    draining: bool,
    assembled_bytes: usize,
    /// Text the wire emitted before any truncation signal, kept so the typed
    /// error can carry the partial answer without re-walking emitted events.
    text: String,
}
impl State {
    fn reserve(&mut self, bytes: usize) -> Result<(), ProviderError> {
        let next = self
            .assembled_bytes
            .checked_add(bytes)
            .ok_or_else(size_error)?;
        if next > crate::MAX_STREAM_BYTES {
            return Err(size_error());
        }
        self.assembled_bytes = next;
        Ok(())
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        if self.bytes.len().saturating_add(chunk.len()) > crate::MAX_STREAM_BYTES {
            return Err(ProviderError::Request(
                "provider stream exceeded size limit".into(),
            ));
        }
        self.bytes.extend_from_slice(chunk);
        let mut consumed = 0;
        while let Some((end, skip)) = boundary(&self.bytes[consumed..]) {
            let usage = self.usage_reported.then_some(self.usage);
            self.frames.admit(usage)?;
            let frame_end = consumed + end;
            let frame = std::str::from_utf8(&self.bytes[consumed..frame_end])
                .map_err(|_| ProviderError::InvalidResponse)?;
            consumed = frame_end + skip;
            let mut data = String::new();
            let mut has_data = false;
            for value in frame.lines().filter_map(|line| line.strip_prefix("data:")) {
                if has_data {
                    data.push('\n');
                }
                data.push_str(value.trim_start());
                has_data = true;
            }
            if !has_data || data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                self.done = true;
                break;
            }
            let chunk: Chunk =
                serde_json::from_str(&data).map_err(|_| ProviderError::InvalidResponse)?;
            let current_usage_reported = chunk.usage.as_ref().is_some_and(has_reported_usage);
            if let Some(usage) = chunk.usage
                && current_usage_reported
            {
                self.usage_reported = true;
                apply_usage(&mut self.usage, &usage);
                self.pending.push_back(ProviderEvent::Usage(self.usage));
            }
            if self.draining {
                // After a truncation only usage matters: the content, the
                // tool deltas and an empty `choices` list are all ignored.
                continue;
            }
            let Some(choice) = chunk.choices.into_iter().next() else {
                // The trailing usage-only chunk carries an empty choices list.
                if !current_usage_reported {
                    return Err(ProviderError::InvalidResponse);
                }
                continue;
            };
            if let Some(reason) = choice.finish_reason.as_deref()
                && !matches!(reason, "stop" | "tool_calls")
            {
                // `length` is deterministic (raise the output cap); anything
                // else stays a generic protocol failure. Truncation is typed
                // and carries the partial output, never retried upstream.
                if reason == "length" {
                    self.deferred = Some(ProviderError::output_truncated(
                        std::mem::take(&mut self.text),
                        self.tools.partial_json(),
                    ));
                    self.draining = true;
                    continue;
                }
                return Err(ProviderError::InvalidResponse);
            }
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                self.reserve(text.len())?;
                self.content = true;
                self.text.push_str(&text);
                self.pending.push_back(ProviderEvent::TextDelta(text));
            }
            if let Some(reasoning) = choice.delta.reasoning_content
                && !reasoning.is_empty()
            {
                self.reserve(reasoning.len())?;
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
                if self.assembled_bytes.saturating_add(self.tools.bytes()) > crate::MAX_STREAM_BYTES
                {
                    return Err(size_error());
                }
            }
        }
        if consumed > 0 {
            self.bytes.drain(..consumed);
        }
        if self.done && !self.draining {
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

fn size_error() -> ProviderError {
    ProviderError::Request("provider stream exceeded size limit".into())
}

fn has_reported_usage(usage: &super::openai_chunks::Usage) -> bool {
    usage.prompt_tokens.is_some()
        || usage.completion_tokens.is_some()
        || usage.prompt_tokens_details.as_ref().is_some_and(|details| {
            details.cached_tokens.is_some() || details.cache_write_tokens.is_some()
        })
        || usage
            .completion_tokens_details
            .as_ref()
            .is_some_and(|details| details.reasoning_tokens.is_some())
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
    accumulated.cache_creation_input_tokens = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cache_write_tokens);
    accumulated.reasoning_tokens = usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens);
}

#[cfg(test)]
#[path = "openai_stream/tests.rs"]
mod tests;
