use crate::{ChatRequest, ReasoningEffort};
use serde_json::{Value, json};
use std::collections::HashMap;

pub(super) fn build_body(
    request: ChatRequest,
    max_output_tokens: Option<u32>,
    temperature: Option<f32>,
) -> Value {
    let mut system_prompts = Vec::new();
    let mut contents = Vec::new();
    let mut tool_name_map = HashMap::new();

    for msg in request.messages {
        if msg.role == "system" {
            if !msg.content.is_empty() {
                system_prompts.push(msg.content);
            }
            continue;
        }

        if msg.role == "assistant" {
            for call in &msg.tool_calls {
                tool_name_map.insert(call.id.clone(), call.name.clone());
            }

            let mut parts = Vec::new();
            if !msg.content.is_empty() {
                parts.push(json!({ "text": msg.content }));
            }
            for call in msg.tool_calls {
                parts.push(json!({
                    "functionCall": {
                        "name": call.name,
                        "args": call.arguments
                    }
                }));
            }
            contents.push(json!({
                "role": "model",
                "parts": parts
            }));
            continue;
        }

        if msg.role == "tool" {
            let id_str = msg.tool_call_id.as_deref().unwrap_or("");
            let resolved_name = tool_name_map
                .get(id_str)
                .map(String::as_str)
                .unwrap_or(id_str);
            contents.push(json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": resolved_name,
                        "response": {
                            // A `functionResponse` result is a JSON value, not
                            // content blocks: an image cannot ride it, and the
                            // text says so rather than dropping it silently.
                            "result": super::images::text_with_note(&msg.content, &msg.images)
                        }
                    }
                }]
            }));
            continue;
        }

        contents.push(json!({
            "role": "user",
            "parts": [{ "text": msg.content }]
        }));
    }

    // An unset ceiling sends no `maxOutputTokens`, so the model's own default
    // applies.
    let mut generation_config = json!({});
    if let Some(max_output_tokens) = max_output_tokens {
        generation_config["maxOutputTokens"] = json!(max_output_tokens);
    }
    if let Some(temperature) = temperature {
        generation_config["temperature"] = json!(temperature);
    }
    // Gemini's effort lever is a thinking token budget. `0` disables thinking
    // (Gemini API docs); `Minimal` maps to `0` (the least-thinking lever Gemini
    // offers), then conservative steps — Low=1024, Medium=4096, High=8192.
    // `Default` sends nothing so the endpoint's own configuration wins.
    if request.reasoning_effort != ReasoningEffort::Default {
        let thinking_budget = match request.reasoning_effort {
            ReasoningEffort::Minimal => 0,
            ReasoningEffort::Low => 1024,
            ReasoningEffort::Medium => 4096,
            ReasoningEffort::High => 8192,
            ReasoningEffort::Default => 0,
        };
        generation_config["thinkingConfig"] = json!({ "thinkingBudget": thinking_budget });
    }
    let mut body = json!({
        "contents": contents,
        "generationConfig": generation_config
    });

    if !system_prompts.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{ "text": system_prompts.join("\n\n") }]
        });
    }

    if !request.tools.is_empty() {
        let declarations: Vec<Value> = request
            .tools
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters
                })
            })
            .collect();
        body["tools"] = json!([{ "functionDeclarations": declarations }]);
    }

    body
}

#[cfg(test)]
#[path = "gemini_request/tests.rs"]
mod tests;
