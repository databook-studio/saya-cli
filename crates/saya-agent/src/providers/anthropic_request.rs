use crate::{ChatRequest, ReasoningEffort};
use serde_json::{Value, json};

/// Anthropic's documented floor for `thinking.budget_tokens`; a smaller budget
/// is refused, so a ceiling with no room for it means no thinking at all.
const MIN_THINKING_BUDGET: u32 = 1024;

/// Where the effort on a request came from. The loop's adaptive policy is
/// deliberately distinct from an effort the operator pinned in configuration:
/// the policy is an internal hint, so its `low` uses the provider's own
/// adaptive control, while an explicit `low` keeps the fixed budget it always
/// sent.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EffortSource {
    /// Chosen by the loop's per-turn policy.
    Policy,
    /// Stated by the caller or pinned by the operator.
    Stated,
}

/// Whether this effort is sent as adaptive thinking. Adaptive rejects
/// temperature and carries `output_config.effort` instead of a budget.
///
/// `High` and the policy's `Low` have used the adaptive control since base.
/// The policy's new `Medium` tier does too, but only on the OpenRouter route:
/// the native route keeps the base behaviour of a fixed budget for `Medium`.
pub(super) fn uses_adaptive(
    effort: ReasoningEffort,
    source: EffortSource,
    openrouter: bool,
) -> bool {
    matches!(effort, ReasoningEffort::High)
        || (effort == ReasoningEffort::Low && source == EffortSource::Policy)
        || (effort == ReasoningEffort::Medium && source == EffortSource::Policy && openrouter)
}

/// The `output_config.effort` spelling of a request's effort.
fn effort_name(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::High => "high",
        ReasoningEffort::Medium => "medium",
        _ => "low",
    }
}

/// The explicit thinking budget for an effort the adaptive control does not
/// cover.
fn thinking_budget(effort: ReasoningEffort) -> Option<u32> {
    match effort {
        ReasoningEffort::Minimal => Some(MIN_THINKING_BUDGET),
        ReasoningEffort::Low => Some(2048),
        ReasoningEffort::Medium => Some(4096),
        ReasoningEffort::High | ReasoningEffort::Default => None,
    }
}

pub(super) fn build_body(
    request: ChatRequest,
    max_tokens: u32,
    temperature: Option<f32>,
    source: EffortSource,
    openrouter: bool,
) -> Value {
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
            // Anthropic's `tool_result.content` takes text and image blocks, so
            // an image rides the result it came from. A result with no image
            // keeps the bare string form it always had.
            let content = if msg.images.is_empty() {
                json!(msg.content)
            } else {
                let mut blocks = vec![json!({"type": "text", "text": msg.content})];
                blocks.extend(msg.images.iter().map(|image| {
                    json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": image.media_type,
                            "data": image.data_base64
                        }
                    })
                }));
                json!(blocks)
            };
            pending_tool_results.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": content
            }));
            continue;
        }

        flush_tool_results(&mut pending_tool_results, &mut messages);

        if msg.role == "assistant" {
            // Anthropic accepts thinking back only as the signed block it
            // issued, so replay needs both halves; unsigned reasoning stays
            // out of the request rather than being guessed at.
            let thinking = match (msg.reasoning.as_deref(), msg.reasoning_signature.as_deref()) {
                (Some(text), Some(signature)) if !text.is_empty() => Some(json!({
                    "type": "thinking",
                    "thinking": text,
                    "signature": signature
                })),
                _ => None,
            };
            if thinking.is_none() && msg.tool_calls.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "content": msg.content
                }));
            } else {
                let mut content = Vec::new();
                if let Some(block) = thinking {
                    content.push(block);
                }
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
        // but a block can. The block's text is the same string the bare-string form
        // produced (`system_prompts.join("\n\n")`) — only the envelope changes,
        // never the content. Marking the system block caches the stable system
        // prefix across turns of a session.
        body["system"] = json!([{
            "type": "text",
            "text": system_prompts.join("\n\n"),
            "cache_control": {"type": "ephemeral"}
        }]);
    }

    // Thinking rejects a modified temperature, so temperature goes out only
    // when no thinking control does.
    let adaptive = uses_adaptive(request.reasoning_effort, source, openrouter);
    let budget_tokens = if adaptive {
        None
    } else {
        thinking_budget(request.reasoning_effort)
            // Anthropic requires the budget to sit strictly below `max_tokens`
            // and rejects the call otherwise, so cap it to the room available.
            // A ceiling too low for the minimum budget cannot carry thinking
            // at all — omit the field rather than send a request the endpoint
            // will refuse.
            .map(|asked| asked.min(max_tokens.saturating_sub(1)))
            .filter(|budget| *budget >= MIN_THINKING_BUDGET)
    };
    if !adaptive && let Some(temperature) = temperature {
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

    // Adaptive effort requests let the model think only as it needs to,
    // instead of a fixed token budget: `high` always, and the policy's `low`
    // with a matching effort setting. Stated lower efforts keep their budget.
    // `Default` sends nothing so the endpoint's own configuration wins.
    if adaptive {
        body["thinking"] = json!({ "type": "adaptive" });
        body["output_config"] = json!({ "effort": effort_name(request.reasoning_effort) });
    } else if let Some(budget_tokens) = budget_tokens {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget_tokens });
    }

    body
}

#[cfg(test)]
#[path = "anthropic_request/openrouter_tests.rs"]
mod openrouter_tests;

#[cfg(test)]
#[path = "anthropic_request/tests.rs"]
mod tests;
