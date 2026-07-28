use super::{ollama_chunks::Chunk, tool_assembly::ToolAssembly};
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
    pending: VecDeque<ProviderEvent>,
    tools: ToolAssembly,
    content: bool,
    done: bool,
}
impl State {
    fn finish(&mut self) -> Result<(), ProviderError> {
        if self.done {
            return Ok(());
        }
        let line = std::str::from_utf8(&self.bytes)
            .map_err(|_| ProviderError::InvalidResponse)?
            .to_owned();
        self.bytes.clear();
        self.push_line(line.trim_end_matches('\r'))?;
        if self.done {
            Ok(())
        } else {
            Err(ProviderError::InvalidResponse)
        }
    }
    fn push(&mut self, chunk: &[u8]) -> Result<(), ProviderError> {
        self.bytes.extend_from_slice(chunk);
        while let Some(end) = self.bytes.iter().position(|byte| *byte == b'\n') {
            let line = String::from_utf8(self.bytes[..end].to_vec())
                .map_err(|_| ProviderError::InvalidResponse)?;
            self.bytes.drain(..=end);
            self.push_line(line.trim_end_matches('\r'))?;
            if self.done {
                break;
            }
        }
        Ok(())
    }
    fn push_line(&mut self, line: &str) -> Result<(), ProviderError> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let chunk: Chunk =
            serde_json::from_str(line).map_err(|_| ProviderError::InvalidResponse)?;
        if let Some(message) = chunk.message {
            if !message.content.is_empty() {
                self.content = true;
                self.pending
                    .push_back(ProviderEvent::TextDelta(message.content));
            }
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
            }
        }
        if chunk.done {
            self.done = true;
            self.complete()?;
        }
        Ok(())
    }
    fn complete(&mut self) -> Result<(), ProviderError> {
        if !self.bytes.is_empty() || (!self.content && self.tools.is_empty()) {
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
