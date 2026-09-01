use super::framing::whitespace;
use crate::{
    CancellationToken, ProviderError, ProviderEvent, ProviderStream, TokenUsage, ToolCall,
};
use futures_util::{StreamExt, stream};
use reqwest::Response;
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

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
    usage: TokenUsage,
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
                    } else if cb_type == "thinking" {
                        // A `thinking` content block can carry its initial text
                        // inline on `content_block_start` (the S20 "whole"
                        // spelling). Emit it as reasoning; the subsequent
                        // `thinking_delta`s append to it.
                        if let Some(thinking) = json["content_block"]["thinking"]
                            .as_str()
                            .filter(|text| !text.is_empty())
                        {
                            self.pending
                                .push_back(ProviderEvent::ReasoningDelta(thinking.to_string()));
                        }
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
                    } else if delta_type == "thinking_delta" {
                        // The streamed reasoning increment (S20 wire table:
                        // `thinking_delta.thinking`). Forwarded as a
                        // `ReasoningDelta` for `collect()` to accumulate.
                        let thinking = json["delta"]["thinking"]
                            .as_str()
                            .ok_or(ProviderError::InvalidResponse)?;
                        if !thinking.is_empty() {
                            self.pending
                                .push_back(ProviderEvent::ReasoningDelta(thinking.to_string()));
                        }
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
                "message_start" => {
                    let usage = &json["message"]["usage"];
                    if apply_usage(&mut self.usage, usage) {
                        self.pending.push_back(ProviderEvent::Usage(self.usage));
                    }
                }
                "message_delta" => {
                    let usage = &json["usage"];
                    if apply_usage(&mut self.usage, usage) {
                        self.pending.push_back(ProviderEvent::Usage(self.usage));
                    }
                }
                "error" => {
                    return Err(ProviderError::InvalidResponse);
                }
                "ping" | "content_block_stop" => {}
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

/// Folds an Anthropic `usage` object (from `message_start` or `message_delta`)
/// into the running totals for one response. Anthropic reports two cache
/// numbers that bill differently and are kept distinct: reads
/// (`cache_read_input_tokens`, inclusive of `input_tokens`) and creation
/// (`cache_creation_input_tokens`, also inclusive of `input_tokens`). Returns
/// whether anything was reported, so the caller only emits a `Usage` event when
/// the object actually carried counts. A reported `0` stays `Some(0)`; an
/// omitted field stays `None` (absent is not zero).
fn apply_usage(accumulated: &mut TokenUsage, usage: &serde_json::Value) -> bool {
    let mut changed = false;
    if let Some(input) = usage["input_tokens"].as_u64() {
        accumulated.input_tokens = input;
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
    changed
}

#[cfg(test)]
mod tests {
    use super::{TokenUsage, apply_usage};
    use serde_json::json;

    /// Deliverable 4 (Anthropic, with): a `message_start` usage object
    /// carrying both cache numbers populates the two cache fields (distinct,
    /// because reads and creation bill differently — Q2).
    #[test]
    fn message_start_with_cache_fields_populates_both() {
        let mut accumulated = TokenUsage::default();
        let usage = json!({
            "input_tokens": 100,
            "cache_read_input_tokens": 90,
            "cache_creation_input_tokens": 10
        });
        assert!(apply_usage(&mut accumulated, &usage));
        assert_eq!(accumulated.input_tokens, 100);
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
            input_tokens: 100,
            cached_input_tokens: Some(90),
            ..Default::default()
        };
        let usage = json!({"output_tokens": 34});
        assert!(apply_usage(&mut accumulated, &usage));
        assert_eq!(accumulated.output_tokens, 34);
        // The earlier cache read survives; `message_delta` did not report it.
        assert_eq!(accumulated.cached_input_tokens, Some(90));
        assert_eq!(accumulated.cache_creation_input_tokens, None);
    }

    /// Deliverable 5 (Anthropic): a reported `cache_read_input_tokens: 0` (a
    /// cache that was created but read nothing) stays `Some(0)`, not `None`.
    #[test]
    fn reported_zero_cache_read_is_some_zero() {
        let mut accumulated = TokenUsage::default();
        let usage = json!({"input_tokens": 10, "cache_read_input_tokens": 0});
        apply_usage(&mut accumulated, &usage);
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
        assert!(!apply_usage(&mut accumulated, &json!({})));
        assert_eq!(accumulated.input_tokens, 7);
    }
}
