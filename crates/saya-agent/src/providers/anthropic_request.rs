use crate::{ChatRequest, ReasoningEffort};
use serde_json::{Value, json};

/// Anthropic's documented floor for `thinking.budget_tokens`; a smaller budget
/// is refused, so a ceiling with no room for it means no thinking at all.
const MIN_THINKING_BUDGET: u32 = 1024;

pub(super) fn build_body(request: ChatRequest, max_tokens: u32, temperature: Option<f32>) -> Value {
    let mut system_prompts = Vec::new();
    let mut messages = Vec::new();
    let mut pending_tool_results = Vec::new();

    let flush_tool_results = |pending: &mut Vec<Value>, msgs: &mut Vec<Value>| {
        if !pending.is_empty() {
            msgs.push(json!({
                "role": "user",
                "content": std::mem::take(pending)
            }));
        }
    };

    for msg in request.messages {
        if msg.role == "system" {
            if !msg.content.is_empty() {
                system_prompts.push(msg.content);
            }
            continue;
        }

        if msg.role == "tool" {
            let tool_use_id = msg.tool_call_id.as_deref().unwrap_or("");
            pending_tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": msg.content
            }));
            continue;
        }

        flush_tool_results(&mut pending_tool_results, &mut messages);

        if msg.role == "assistant" {
            if msg.tool_calls.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "content": msg.content
                }));
            } else {
                let mut content = Vec::new();
                if !msg.content.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": msg.content
                    }));
                }
                for call in msg.tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": content
                }));
            }
        } else {
            messages.push(json!({
                "role": msg.role,
                "content": msg.content
            }));
        }
    }

    flush_tool_results(&mut pending_tool_results, &mut messages);

    let mut body = json!({
        "model": request.model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": messages
    });

    if !system_prompts.is_empty() {
        // `system` is sent as a single-element array of content blocks rather
        // than a bare string: a string can't carry a `cache_control` breakpoint,
        // but a block can. The block's text is the same string the old form
        // produced (`system_prompts.join("\n\n")`) — only the envelope changes,
        // never the content. Marking the system block caches the stable system
        // prefix across turns of a session.
        body["system"] = json!([{
            "type": "text",
            "text": system_prompts.join("\n\n"),
            "cache_control": {"type": "ephemeral"}
        }]);
    }

    if let Some(temperature) = temperature {
        body["temperature"] = json!(temperature);
    }

    if !request.tools.is_empty() {
        let mut tools: Vec<Value> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters
                })
            })
            .collect();
        // Tools are the largest stable block in the request and never change
        // within a session, so the highest-value cache breakpoint is on the
        // **last** tool: everything from the start of the prompt up to and
        // including it is cached. One breakpoint covers the whole tool list.
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = json!({"type": "ephemeral"});
        }
        body["tools"] = json!(tools);
    }

    // Anthropic's effort lever is a thinking token budget. Each level doubles
    // from the documented minimum —
    // Minimal=1024, Low=2048, Medium=4096, High=8192 — so a higher effort asks for
    // proportionally more thinking. `Default` sends nothing so the endpoint's own
    // configuration wins.
    if request.reasoning_effort != ReasoningEffort::Default {
        let asked = match request.reasoning_effort {
            ReasoningEffort::Minimal => MIN_THINKING_BUDGET,
            ReasoningEffort::Low => 2048,
            ReasoningEffort::Medium => 4096,
            ReasoningEffort::High => 8192,
            ReasoningEffort::Default => 0,
        };
        // Anthropic requires the budget to sit strictly below `max_tokens` and
        // rejects the call otherwise, so cap it to the room available. A ceiling
        // too low for the minimum budget cannot carry thinking at all — omit the
        // field rather than send a request the endpoint will refuse.
        let budget_tokens = asked.min(max_tokens.saturating_sub(1));
        if budget_tokens >= MIN_THINKING_BUDGET {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget_tokens });
        }
    }

    body
}

#[cfg(test)]
#[path = "anthropic_request/tests.rs"]
mod tests;
