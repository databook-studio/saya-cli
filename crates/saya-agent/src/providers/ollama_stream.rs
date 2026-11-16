use super::{
    framing::whitespace, ollama_chunks::Chunk, stall::ProgressClock, tool_assembly::ToolAssembly,
};
use crate::{CancellationToken, ProviderError, ProviderEvent, ProviderStream, TokenUsage};
use futures_util::{StreamExt, stream};
use reqwest::Response;
use std::{collections::VecDeque, time::Duration};

pub(super) fn parse(
    response: Response,
    cancellation: CancellationToken,
    idle: Duration,
    request_timeout: Duration,
    stall: Duration,
) -> ProviderStream {
    Box::pin(stream::unfold(
        (
            response.bytes_stream(),
            State {
                progress: ProgressClock::new(stall),
                ..State::default()
            },
            cancellation,
            idle,
            std::time::Instant::now(),
            request_timeout,
        ),
        next,
    ))
}

async fn next<S>(
    mut value: (
        S,
        State,
        CancellationToken,
        Duration,
        std::time::Instant,
        Duration,
    ),
) -> Option<(
    Result<ProviderEvent, ProviderError>,
    (
        S,
        State,
        CancellationToken,
        Duration,
        std::time::Instant,
        Duration,
    ),
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
        if let Some(error) = value.1.deferred.take() {
            value.1.done = true;
            return Some((Err(error), value));
        }
        if value.1.done {
            return None;
        }
        // Per-chunk idle budget, content-progress budget, and the total
        // request deadline (see anthropic_stream).
        let stall_at = tokio::time::Instant::from_std(value.1.progress.deadline(value.4));
        let item = tokio::select! {
            _ = value.2.cancelled() => return Some((Err(ProviderError::Cancelled), value)),
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(value.4 + value.5)) => {
                value.1.done = true;
                let visible = value.1.text.len().saturating_add(value.1.tools.bytes());
                return Some((
                    Err(super::framing::request_cut(value.5, visible)),
                    value,
                ));
            }
            _ = tokio::time::sleep_until(stall_at) => {
                value.1.done = true;
                return Some((Err(ProviderError::StreamStalled), value));
            }
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
            if let Err(error) = value.1.finish() {
                value.1.done = true;
                return Some((Err(error), value));
            }
            continue;
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
    frames: super::framing::FrameCount,
    usage: TokenUsage,
    usage_reported: bool,
    pending: VecDeque<ProviderEvent>,
    /// Truncation error held back until the driver has drained `pending`.
    deferred: Option<ProviderError>,
    tools: ToolAssembly,
    content: bool,
    done: bool,
    assembled_bytes: usize,
    /// Text the wire emitted before any truncation signal, kept so the typed
    /// error can carry the partial answer without re-walking emitted events.
    text: String,
    /// Last content-bearing delta, so keep-alives cannot hold a silent stream
    /// open.
    progress: ProgressClock,
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

    fn finish(&mut self) -> Result<(), ProviderError> {
        if self.done {
            return if whitespace(&self.bytes) {
                self.bytes.clear();
                Ok(())
            } else {
                Err(ProviderError::InvalidResponse)
            };
        }
        let bytes = std::mem::take(&mut self.bytes);
        self.frames
            .admit(self.usage_reported.then_some(self.usage))?;
        let line = std::str::from_utf8(&bytes).map_err(|_| ProviderError::InvalidResponse)?;
        self.push_line(line.trim_end_matches('\r'), &[])?;
        if self.done {
            Ok(())
        } else {
            Err(ProviderError::InvalidResponse)
        }
    }
    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        if self.bytes.len().saturating_add(chunk.len()) > crate::MAX_STREAM_BYTES {
            return Err(ProviderError::Request(
                "provider stream exceeded size limit".into(),
            ));
        }
        self.bytes.extend_from_slice(chunk);
        let bytes = std::mem::take(&mut self.bytes);
        let mut consumed = 0;
        while let Some(relative_end) = bytes[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + relative_end;
            self.frames
                .admit(self.usage_reported.then_some(self.usage))?;
            let line = std::str::from_utf8(&bytes[consumed..end])
                .map_err(|_| ProviderError::InvalidResponse)?;
            consumed = end + 1;
            self.push_line(line.trim_end_matches('\r'), &bytes[consumed..])?;
            if self.done {
                break;
            }
        }
        if !self.done {
            self.bytes.extend_from_slice(&bytes[consumed..]);
        }
        Ok(())
    }
    fn push_line(&mut self, line: &str, trailing: &[u8]) -> Result<(), ProviderError> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let chunk: Chunk =
            serde_json::from_str(line).map_err(|_| ProviderError::InvalidResponse)?;
        let mut usage = self.usage;
        let mut usage_reported = self.usage_reported;
        if chunk.done {
            if let Some(input) = chunk.prompt_eval_count {
                usage.input_tokens = input;
                usage_reported = true;
            }
            if let Some(output) = chunk.eval_count {
                usage.output_tokens = output;
                usage_reported = true;
            }
            if let Some(cached) = chunk.prompt_eval_cached_count {
                usage.cached_input_tokens = Some(cached);
                usage_reported = true;
            }
        }
        if let Some(message) = chunk.message {
            if !message.content.is_empty() {
                self.reserve(message.content.len())?;
                self.content = true;
                self.text.push_str(&message.content);
                self.progress.observe();
                self.pending
                    .push_back(ProviderEvent::TextDelta(message.content));
            }
            if !message.thinking.is_empty() {
                self.reserve(message.thinking.len())?;
                self.progress.observe();
                self.pending
                    .push_back(ProviderEvent::ReasoningDelta(message.thinking));
            }
            let tool_delta = !message.tool_calls.is_empty();
            for (index, call) in message.tool_calls.into_iter().enumerate() {
                let arguments = match call.function.arguments {
                    serde_json::Value::String(value) => value,
                    value => value.to_string(),
                };
                self.tools.push(
                    index,
                    call.id.as_deref(),
                    Some(&call.function.name),
                    Some(&arguments),
                )?;
                if self.assembled_bytes.saturating_add(self.tools.bytes()) > crate::MAX_STREAM_BYTES
                {
                    return Err(size_error());
                }
            }
            if tool_delta {
                self.progress.observe();
            }
        }
        if chunk.done {
            self.done = true;
            self.usage = usage;
            self.usage_reported = usage_reported;
            if self.usage_reported {
                self.pending.push_back(ProviderEvent::Usage(self.usage));
            }
            if chunk.done_reason.as_deref() == Some("length") {
                self.deferred = Some(ProviderError::output_truncated(
                    std::mem::take(&mut self.text),
                    self.tools.partial_json(),
                ));
                return Ok(());
            }
            self.complete(trailing)?;
        }
        Ok(())
    }
    fn complete(&mut self, trailing: &[u8]) -> Result<(), ProviderError> {
        if !whitespace(trailing) || (!self.content && self.tools.is_empty()) {
            return Err(ProviderError::InvalidResponse);
        }
        let calls = std::mem::take(&mut self.tools).finish()?;
        if !calls.is_empty() {
            self.pending.push_back(ProviderEvent::ToolCalls(calls));
        }
        self.pending.push_back(ProviderEvent::Done);
        Ok(())
    }
}

fn size_error() -> ProviderError {
    ProviderError::Request("provider stream exceeded size limit".into())
}
