use super::{
    anthropic_stream::{State, apply_usage, boundary, size_error},
    framing::whitespace,
};
use crate::{ProviderError, ProviderEvent};

pub(super) fn push(state: &mut State, chunk: &[u8]) -> Result<(), ProviderError> {
    if state.bytes.len().saturating_add(chunk.len()) > crate::MAX_STREAM_BYTES {
        return Err(size_error());
    }
    state.bytes.extend_from_slice(chunk);
    while let Some((end, skip)) = boundary(&state.bytes) {
        let frame = String::from_utf8(state.bytes[..end].to_vec())
            .map_err(|_| ProviderError::InvalidResponse)?;
        state.bytes.drain(..end + skip);
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
        match json["type"]
            .as_str()
            .ok_or(ProviderError::InvalidResponse)?
        {
            "content_block_start" => start(state, &json)?,
            "content_block_delta" => delta(state, &json)?,
            "message_stop" => finish(state)?,
            "message_start" => usage(state, &json["message"]["usage"]),
            "message_delta" => {
                usage(state, &json["usage"]);
                if let Some(reason) = json["delta"]["stop_reason"].as_str() {
                    state.stop_reason = Some(reason.into());
                }
            }
            "error" => return Err(ProviderError::InvalidResponse),
            "ping" | "content_block_stop" => {}
            _ => {}
        }
        if state.done {
            break;
        }
    }
    if state.done {
        if !whitespace(&state.bytes) {
            return Err(ProviderError::InvalidResponse);
        }
        state.bytes.clear();
    }
    Ok(())
}

fn start(state: &mut State, json: &serde_json::Value) -> Result<(), ProviderError> {
    let block = &json["content_block"];
    if block["type"]
        .as_str()
        .ok_or(ProviderError::InvalidResponse)?
        == "tool_use"
    {
        state.tools.start(
            index(json)?,
            block["id"].as_str().unwrap_or(""),
            block["name"].as_str().unwrap_or(""),
        )?;
        return tool_bytes(state);
    }
    if let Some(thinking) = block["thinking"].as_str().filter(|text| !text.is_empty()) {
        state.reserve(thinking.len())?;
        state
            .pending
            .push_back(ProviderEvent::ReasoningDelta(thinking.into()));
    }
    Ok(())
}

fn delta(state: &mut State, json: &serde_json::Value) -> Result<(), ProviderError> {
    let delta = &json["delta"];
    match delta["type"]
        .as_str()
        .ok_or(ProviderError::InvalidResponse)?
    {
        "text_delta" => {
            let text = delta["text"]
                .as_str()
                .ok_or(ProviderError::InvalidResponse)?;
            state.reserve(text.len())?;
            state.text.push_str(text);
            state
                .pending
                .push_back(ProviderEvent::TextDelta(text.into()));
        }
        "thinking_delta" => {
            let thinking = delta["thinking"]
                .as_str()
                .ok_or(ProviderError::InvalidResponse)?;
            state.reserve(thinking.len())?;
            if !thinking.is_empty() {
                state
                    .pending
                    .push_back(ProviderEvent::ReasoningDelta(thinking.into()));
            }
        }
        "input_json_delta" => {
            let partial = delta["partial_json"]
                .as_str()
                .ok_or(ProviderError::InvalidResponse)?;
            state.tools.append_arguments(index(json)?, partial)?;
            tool_bytes(state)?;
        }
        _ => {}
    }
    Ok(())
}

fn finish(state: &mut State) -> Result<(), ProviderError> {
    if state.stop_reason.as_deref() == Some("max_tokens") {
        return Err(ProviderError::output_truncated(
            std::mem::take(&mut state.text),
            state.tools.partial_json(),
        ));
    }
    let calls = std::mem::take(&mut state.tools).finish_with_empty_object()?;
    if !calls.is_empty() {
        state.pending.push_back(ProviderEvent::ToolCalls(calls));
    }
    state.pending.push_back(ProviderEvent::Done);
    state.done = true;
    Ok(())
}

fn usage(state: &mut State, value: &serde_json::Value) {
    if apply_usage(&mut state.usage, value) {
        state.pending.push_back(ProviderEvent::Usage(state.usage));
    }
}

fn index(json: &serde_json::Value) -> Result<usize, ProviderError> {
    json["index"]
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(ProviderError::InvalidResponse)
}

fn tool_bytes(state: &State) -> Result<(), ProviderError> {
    state
        .assembled_bytes
        .checked_add(state.tools.bytes())
        .filter(|bytes| *bytes <= crate::MAX_STREAM_BYTES)
        .map(|_| ())
        .ok_or(ProviderError::ToolCollectionLimit)
}
