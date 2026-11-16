//! The clarification ask as one shared tool: the definition every
//! surface advertises and the short result a directly-reached call answers
//! with.
//!
//! The ask is not a database tool. It touches no data, no local state and
//! nothing external, so it needs no connection and no profile — a surface
//! admits it wherever an operator can answer it: the chat toolset always, the
//! coding surfaces only when an operator is attached (a headless coding job
//! must never stop for a question nobody can answer). The loop intercepts the
//! call before the ordinary tool path and ends the turn on it; see
//! `saya-agent`'s `loop_runner::clarification`.

use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};

/// Whether `name` is the clarification ask.
pub(crate) fn is_ask(name: &str) -> bool {
    name == saya_agent::REQUEST_CLARIFICATION_TOOL
}

/// The one `request_clarification` definition. The bounds in the schema come
/// from the same constants the loop enforces, so the advertised shape and the
/// enforced one cannot drift.
pub(crate) fn definition() -> ToolDefinition {
    ToolDefinition {
        name: saya_agent::REQUEST_CLARIFICATION_TOOL.into(),
        description: "Stop and ask the user one focused question when a material definition — \
            a grain, a time column, a metric, a filter — is ambiguous and nothing confirmed \
            in this session resolves it. Ask instead of assuming: a wrong assumption \
            produces a confident wrong answer, and the user would rather answer once. \
            Pass `question` — one question, at most 300 characters — and optionally \
            `options`, up to six candidate answers of at most 80 characters each, so the \
            user can answer by number. Calling this ends your turn: the user's next \
            message is the answer, and nothing else runs until then. Never bundle other \
            tool calls with it."
            .into(),
        read_only: true,
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The one focused question to ask, at most 300 characters."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": saya_agent::MAX_CLARIFICATION_OPTIONS,
                    "description": "Optional. Up to six candidate answers, each at most 80 characters."
                }
            },
            "required": ["question"],
            "additionalProperties": false
        }),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("question asked — the turn pauses for the user's answer".into()),
    }
}

/// The result a directly-reached ask answers with: the loop ends the turn on
/// the call, so this is what the model reads in the only tool result of that
/// turn. It records nothing — an ask is not evidence about anything.
pub(crate) fn result() -> serde_json::Value {
    serde_json::json!({
        "asked": true,
        "next": "stop your turn here; the user will answer in their next message"
    })
}
