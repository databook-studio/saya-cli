use super::framing::whitespace;
use crate::{CancellationToken, ProviderError, ProviderEvent, ProviderStream, ToolCall};
use futures_util::{StreamExt, stream};
use reqwest::Response;
use std::collections::{BTreeMap, VecDeque};

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
        let item = tokio::select! {
            _ = value.2.cancelled() => return Some((Err(ProviderError::Cancelled), value)),
            item = value.0.next() => item
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
struct ToolUseBlock {
    id: String,
    name: String,
    json: String,
}

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    pending: VecDeque<ProviderEvent>,
    tools: BTreeMap<usize, ToolUseBlock>,
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
            let json: serde_json::Value =
                serde_json::from_str(&data).map_err(|_| ProviderError::InvalidResponse)?;

            let event_type = json["type"]
                .as_str()
                .ok_or(ProviderError::InvalidResponse)?;
            match event_type {
                "content_block_start" => {
                    let index = json["index"]
                        .as_u64()
                        .ok_or(ProviderError::InvalidResponse)?
                        as usize;
                    let cb_type = json["content_block"]["type"]
                        .as_str()
                        .ok_or(ProviderError::InvalidResponse)?;
                    if cb_type == "tool_use" {
                        let id = json["content_block"]["id"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let name = json["content_block"]["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        self.tools.insert(
                            index,
                            ToolUseBlock {
                                id,
                                name,
                                json: String::new(),
                            },
                        );
                    }
                }
                "content_block_delta" => {
                    let index = json["index"]
                        .as_u64()
                        .ok_or(ProviderError::InvalidResponse)?
                        as usize;
                    let delta_type = json["delta"]["type"]
                        .as_str()
                        .ok_or(ProviderError::InvalidResponse)?;
                    if delta_type == "text_delta" {
                        let text = json["delta"]["text"]
                            .as_str()
                            .ok_or(ProviderError::InvalidResponse)?;
                        self.pending
                            .push_back(ProviderEvent::TextDelta(text.to_string()));
                    } else if delta_type == "input_json_delta" {
                        let partial = json["delta"]["partial_json"]
                            .as_str()
                            .ok_or(ProviderError::InvalidResponse)?;
                        if let Some(block) = self.tools.get_mut(&index) {
                            block.json.push_str(partial);
                        } else {
                            return Err(ProviderError::InvalidResponse);
                        }
                    }
                }
                "message_stop" => {
                    let mut calls = Vec::new();
                    for (_index, block) in std::mem::take(&mut self.tools) {
                        let arguments = if block.json.trim().is_empty() {
                            serde_json::json!({})
                        } else {
                            serde_json::from_str(&block.json)
                                .map_err(|_| ProviderError::InvalidResponse)?
                        };
                        calls.push(ToolCall {
                            id: block.id,
                            name: block.name,
                            arguments,
                        });
                    }
                    if !calls.is_empty() {
                        self.pending.push_back(ProviderEvent::ToolCalls(calls));
                    }
                    self.pending.push_back(ProviderEvent::Done);
                    self.done = true;
                    break;
                }
                "error" => {
                    return Err(ProviderError::InvalidResponse);
                }
                "ping" | "message_start" | "content_block_stop" | "message_delta" => {}
                _ => {}
            }
        }
        if self.done {
            if !whitespace(&self.bytes) {
                return Err(ProviderError::InvalidResponse);
            }
            self.bytes.clear();
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
