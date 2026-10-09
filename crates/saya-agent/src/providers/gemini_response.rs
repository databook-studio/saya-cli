use crate::{ChatMessage, ChatResponse, ProviderError, TokenUsage, ToolCall};
use serde_json::Value;

pub(super) fn parse(body: Value) -> Result<ChatResponse, ProviderError> {
    // A capped response reports `finishReason: "MAX_TOKENS"` on the first
    // candidate. Without this check it would end and look complete — silent
    // truncation, strictly worse than the loud failure. The partial content
    // below is redacted at construction, like every other model output.
    if body["candidates"][0]["finishReason"].as_str() == Some("MAX_TOKENS") {
        let mut partial_text = String::new();
        collect_text(&body, &mut partial_text);
        return Err(ProviderError::output_truncated(partial_text, Vec::new()));
    }
    let parts = body
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .ok_or(ProviderError::InvalidResponse)?;

    if parts.is_empty() {
        return Err(ProviderError::InvalidResponse);
    }

    let mut content = String::new();
    let mut tool_calls = Vec::new();
    // Reasoning parts are accumulated separately from the answer. Gemini marks
    // chain-of-thought with `thought: true` on the part; the
    // answer's parts carry no such flag. A response with no `thought: true`
    // part leaves `reasoning` `None` (absent is not zero), distinct from a
    // model that reasoned and produced an empty string.
    let mut reasoning = None;

    for part in parts {
        let is_thought = part
            .get("thought")
            .and_then(|t| t.as_bool())
            .unwrap_or(false);
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            if is_thought {
                reasoning.get_or_insert_with(String::new).push_str(text);
            } else {
                content.push_str(text);
            }
        }
        if let Some(name) = part
            .get("functionCall")
            .and_then(|fc| fc.get("name"))
            .and_then(|n| n.as_str())
        {
            let fc = &part["functionCall"];
            let args = fc
                .get("args")
                .filter(|a| a.is_object())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            tool_calls.push(ToolCall {
                id: format!("gemini-{name}"),
                name: name.to_string(),
                arguments: args,
            });
        }
    }

    if content.is_empty() && tool_calls.is_empty() {
        return Err(ProviderError::InvalidResponse);
    }

    // Gemini's `complete()` is non-streaming and bypasses `collect()`, so the
    // usage is threaded here, not folded from stream events. `usageMetadata` is
    // optional; a response that omits it leaves `usage` `None` (absent is not
    // zero), mirroring the streaming path's "no event ⇒ no usage" rule.
    let usage = usage(&body);

    Ok(ChatResponse {
        message: ChatMessage {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
        },
        reasoning,
        usage,
    })
}

/// Gathers the non-thought text parts of the first candidate, so a capped
/// response still carries the partial answer on the truncation signal.
fn collect_text(body: &Value, out: &mut String) {
    let Some(parts) = body["candidates"][0]["content"]["parts"].as_array() else {
        return;
    };
    for part in parts {
        let is_thought = part
            .get("thought")
            .and_then(|t| t.as_bool())
            .unwrap_or(false);
        if !is_thought && let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            out.push_str(text);
        }
    }
}

/// Extracts token usage from a Gemini `generateContent` response's/// `usageMetadata`, or `None` when the response carries no `usageMetadata` at
/// all. Gemini reports `promptTokenCount` as input, `candidatesTokenCount` as
/// candidate output, and `thoughtsTokenCount` separately; the adapter adds the
/// last two to form complete generated usage while retaining thoughts as a
/// detail. `cachedContentTokenCount` is a subset of prompt input. A wholly
/// absent `usageMetadata` is `None`, distinct from a present block reporting
/// zeros.
pub(super) fn usage(body: &Value) -> Option<TokenUsage> {
    let metadata = body.get("usageMetadata")?;
    let mut usage = TokenUsage::default();
    if let Some(input) = metadata["promptTokenCount"].as_u64() {
        usage.input_tokens = input;
    }
    let candidates = metadata["candidatesTokenCount"].as_u64();
    let thoughts = metadata["thoughtsTokenCount"].as_u64();
    usage.output_tokens = candidates
        .unwrap_or(0)
        .saturating_add(thoughts.unwrap_or(0));
    if let Some(cached) = metadata["cachedContentTokenCount"].as_u64() {
        usage.cached_input_tokens = Some(cached);
    }
    if let Some(reasoning) = thoughts {
        usage.reasoning_tokens = Some(reasoning);
    }
    Some(usage)
}

#[cfg(test)]
#[path = "gemini_response/tests.rs"]
mod tests;
