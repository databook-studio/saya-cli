use super::{openai_chunks::Chunk, tool_assembly::ToolAssembly};
use crate::{CancellationToken, ProviderError, ProviderEvent, ProviderStream};
use futures_util::{StreamExt, stream};
use reqwest::Response;
use std::collections::VecDeque;

pub(super) fn parse(response: Response, cancellation: CancellationToken) -> ProviderStream {
    Box::pin(stream::unfold(
        (response.bytes_stream(), State::default(), cancellation),
        next,
    ))
}

async fn next<S>(
    mut value: (S, State, CancellationToken),
) -> Option<(
    Result<ProviderEvent, ProviderError>,
    (S, State, CancellationToken),
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
        let item = tokio::select! { _ = value.2.cancelled() => return Some((Err(ProviderError::Cancelled), value)), item = value.0.next() => item };
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
    content: bool,
    done: bool,
}
impl State {
    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
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
            let choice = chunk
                .choices
                .into_iter()
                .next()
                .ok_or(ProviderError::InvalidResponse)?;
            if let Some(reason) = choice.finish_reason.as_deref() {
                if !matches!(reason, "stop" | "tool_calls") {
                    return Err(ProviderError::InvalidResponse);
                }
            }
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    self.content = true;
                    self.pending.push_back(ProviderEvent::TextDelta(text));
                }
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
            if !self.bytes.is_empty() || (!self.content && self.tools.is_empty()) {
                return Err(ProviderError::InvalidResponse);
            }
            let calls = std::mem::take(&mut self.tools).finish()?;
            if !calls.is_empty() {
                self.pending.push_back(ProviderEvent::ToolCalls(calls));
            }
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
