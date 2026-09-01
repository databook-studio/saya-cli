use crate::{ChatMessage, ChatResponse, ProviderError, TokenUsage, ToolCall};
use serde_json::Value;

pub(super) fn parse(body: Value) -> Result<ChatResponse, ProviderError> {
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
    // chain-of-thought with `thought: true` on the part (S20 wire table); the
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

/// Extracts token usage from a Gemini `generateContent` response's
/// `usageMetadata`, or `None` when the response carries no `usageMetadata` at
/// all. Gemini reports `promptTokenCount`/`candidatesTokenCount` (input/output)
/// plus `cachedContentTokenCount` (a cache read, inclusive of the prompt) and
/// `thoughtsTokenCount` (reasoning, **separate** from
/// `candidatesTokenCount`, unlike OpenAI's reasoning figure). Fields a Gemini
/// response omits stay `None` (absent is not zero); it has no cache-creation
/// concept. A wholly absent `usageMetadata` is `None` — distinct from a present
/// block reporting zeros — so a silent Gemini response is not mistaken for a
/// free one (invariant 1).
fn usage(body: &Value) -> Option<TokenUsage> {
    let metadata = body.get("usageMetadata")?;
    let mut usage = TokenUsage::default();
    if let Some(input) = metadata["promptTokenCount"].as_u64() {
        usage.input_tokens = input;
    }
    if let Some(output) = metadata["candidatesTokenCount"].as_u64() {
        usage.output_tokens = output;
    }
    if let Some(cached) = metadata["cachedContentTokenCount"].as_u64() {
        usage.cached_input_tokens = Some(cached);
    }
    if let Some(reasoning) = metadata["thoughtsTokenCount"].as_u64() {
        usage.reasoning_tokens = Some(reasoning);
    }
    Some(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_text_response() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello from Gemini!"}]
                }
            }]
        });

        let response = parse(body).unwrap();
        assert_eq!(response.message.role, "assistant");
        assert_eq!(response.message.content, "Hello from Gemini!");
        assert!(response.message.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_function_call_response() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "weather",
                            "args": {"location": "Paris"}
                        }
                    }]
                }
            }]
        });

        let response = parse(body).unwrap();
        assert_eq!(response.message.role, "assistant");
        assert!(response.message.content.is_empty());
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(response.message.tool_calls[0].id, "gemini-weather");
        assert_eq!(response.message.tool_calls[0].name, "weather");
        assert_eq!(
            response.message.tool_calls[0].arguments,
            json!({"location": "Paris"})
        );
    }

    #[test]
    fn test_parse_invalid_response() {
        assert!(parse(json!({})).is_err());
        assert!(parse(json!({"candidates": []})).is_err());
        assert!(parse(json!({"candidates": [{"content": {"parts": []}}]})).is_err());
    }

    /// S26 deliverable 4 (Gemini override path): Gemini's `complete()` bypasses
    /// `collect()` and returns `gemini_response::parse(value)`, so the usage
    /// must be threaded here — the `let _ = usage(&body);` discard is gone (Q3).
    /// A `usageMetadata` carrying the cache-read and reasoning counts reaches
    /// `response.usage`.
    #[test]
    fn parse_threads_usage_into_chat_response() {
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "hi"}]}}],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 200,
                "cachedContentTokenCount": 90,
                "thoughtsTokenCount": 270
            }
        });
        let response = parse(body).expect("parses");
        let usage = response.usage.expect("usage reached the response");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 200);
        assert_eq!(usage.cached_input_tokens, Some(90));
        assert_eq!(usage.reasoning_tokens, Some(270));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    /// S26 invariant 1 (Gemini override path, absent case): a response with no
    /// `usageMetadata` leaves `response.usage` `None`, distinct from a present
    /// block reporting zeros — a silent Gemini response is not a free one.
    #[test]
    fn parse_leaves_usage_none_when_no_usage_metadata() {
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "hi"}]}}]
        });
        let response = parse(body).expect("parses");
        assert_eq!(response.usage, None);
    }

    /// Deliverable 4 (Gemini, with): `usageMetadata` carrying the cache-read
    /// and reasoning counts populates `cached_input_tokens` and
    /// `reasoning_tokens`. Note Gemini's `thoughtsTokenCount` is separate from
    /// `candidatesTokenCount` (Q1), not inclusive of output.
    #[test]
    fn usage_metadata_populates_cache_and_reasoning() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "hi"}]}
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 200,
                "cachedContentTokenCount": 90,
                "thoughtsTokenCount": 270
            }
        });
        let usage = usage(&body).expect("present usageMetadata yields Some");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 200);
        assert_eq!(usage.cached_input_tokens, Some(90));
        assert_eq!(usage.reasoning_tokens, Some(270));
        assert_eq!(usage.cache_creation_input_tokens, None);
    }

    /// Deliverable 4 (Gemini, without): a response with no `usageMetadata`
    /// yields `None` (absent is not zero — a silent Gemini response is not
    /// mistaken for one that reported zeros).
    #[test]
    fn missing_usage_metadata_is_none() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "hi"}]}
            }]
        });
        assert_eq!(usage(&body), None);
    }

    /// Deliverable 5 (Gemini): a reported `cachedContentTokenCount: 0` stays
    /// `Some(0)` inside the returned usage, distinct from an absent field.
    #[test]
    fn reported_zero_cache_is_some_zero() {
        let body = json!({
            "usageMetadata": {"promptTokenCount": 10, "cachedContentTokenCount": 0}
        });
        let usage = usage(&body).expect("present usageMetadata yields Some");
        assert_eq!(usage.cached_input_tokens, Some(0));
    }

    /// S23 deliverable 6 (Gemini, with): parts marked `thought: true` carry
    /// the chain-of-thought; it reaches `response.reasoning`, separate from
    /// the answer's content. Gemini's `complete()` bypasses `collect()`, so
    /// reasoning is parsed directly here (S23 Q4).
    #[test]
    fn thought_parts_carry_reasoning_separate_from_content() {
        let body = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "the time column looks nullable", "thought": true},
                        {"text": "use return_date"}
                    ]
                }
            }]
        });
        let response = parse(body).expect("parses");
        assert_eq!(response.message.content, "use return_date");
        assert_eq!(
            response.reasoning.as_deref(),
            Some("the time column looks nullable"),
            "thought:true parts are reasoning, not content"
        );
    }

    /// S23 deliverable 6 (Gemini, absent): a response whose parts carry no
    /// `thought: true` flag leaves `response.reasoning` `None`, and content
    /// parses normally — a non-reasoning response is unaffected (invariant 3).
    #[test]
    fn response_without_thought_parts_leaves_reasoning_none() {
        let body = json!({
            "candidates": [{
                "content": {"parts": [{"text": "just an answer"}]}
            }]
        });
        let response = parse(body).expect("parses");
        assert_eq!(response.message.content, "just an answer");
        assert_eq!(response.reasoning, None);
    }
}
