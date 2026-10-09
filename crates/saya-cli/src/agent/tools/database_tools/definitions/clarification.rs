fn append_clarification_tools(tools: &mut Vec<ToolDefinition>) {
    // The clarification ask (B3c) is unconditional, like `schema_discovery`:
    // it touches no data, no local state, and nothing external, so every
    // privacy posture admits it. The bounds in the schema come from the
    // same constants the loop enforces, so the advertised shape and the
    // enforced one cannot drift. The loop intercepts the call before the
    // ordinary tool path and ends the turn on it — see saya-agent's
    // `loop_runner::clarification`.
    tools.push(ToolDefinition {
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
        });
}
