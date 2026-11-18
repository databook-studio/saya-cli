/// One reply the scriptable mock server sends per provider request.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) enum ScriptStep {
    Clarify {
        delay: std::time::Duration,
    },
    Natural,
    /// A natural answer whose usage frame reports `input_tokens` as the
    /// prompt tokens (the figure that becomes the answering input usage).
    NaturalWithUsage {
        input_tokens: u64,
    },
    /// A natural answer with NO usage frame: the numerator stays unknown.
    NaturalNoUsage,
    /// A plain text completion (the summariser's reply shape).
    Summary(String),
    DelayedNatural {
        delay: std::time::Duration,
    },
    /// A tool call the model makes: `name` with `arguments`. The mock assigns
    /// the id `call-golden-<request index>`; the goldens never record it.
    ToolCall {
        name: String,
        arguments: serde_json::Value,
    },
    Harmless,
    HoldOpen {
        marker: String,
    },
}

/// The one scripted clarification: one focused question with two options.
#[cfg(unix)]
pub(crate) fn clarification_arguments() -> serde_json::Value {
    serde_json::json!({
        "question": "Which one?",
        "options": ["a", "b"]
    })
}

#[cfg(unix)]
pub(crate) fn clarification_body(request_index: usize) -> String {
    let call = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": tool_call_id(request_index),
                "function": {
                    "name": "request_clarification",
                    "arguments": clarification_arguments().to_string()
                }
            }]},
            "finish_reason": "tool_calls"
        }]
    });
    format!("data: {call}\n\n")
}

/// The id a scripted tool call gets: unique per request slot, and echoed by
/// the loop on the tool result message so the golden can pair the two.
#[cfg(unix)]
pub(crate) fn tool_call_id(request_index: usize) -> String {
    format!("call-golden-{request_index}")
}

/// One assistant turn carrying a single tool call. The finish reason makes
/// the loop execute the call and continue; the next script slot answers the
/// request that carries the tool result.
#[cfg(unix)]
pub(crate) fn tool_call_body(
    name: &str,
    arguments: &serde_json::Value,
    request_index: usize,
) -> String {
    let call = serde_json::json!({
        "choices": [{
            "delta": {"tool_calls": [{
                "index": 0,
                "id": tool_call_id(request_index),
                "function": {"name": name, "arguments": arguments.to_string()}
            }]},
            "finish_reason": "tool_calls"
        }]
    });
    format!("data: {call}\n\n")
}

#[cfg(unix)]
pub(crate) fn answer_body() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"the-final-answer\"},",
        "\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,",
        "\"completion_tokens\":1}}\n\n"
    )
    .to_owned()
}

#[cfg(unix)]
pub(crate) fn answer_with_usage_body(input_tokens: u64) -> String {
    format!(
        concat!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"the-final-answer\"}},",
            "\"finish_reason\":\"stop\"}}]}}\n\n",
            "data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":{},",
            "\"completion_tokens\":5}}}}\n\n"
        ),
        input_tokens
    )
}

#[cfg(unix)]
pub(crate) fn answer_no_usage_body() -> String {
    concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"the-final-answer\"},",
        "\"finish_reason\":\"stop\"}]}\n\n"
    )
    .to_owned()
}

#[cfg(unix)]
pub(crate) fn summary_body(text: &str) -> String {
    format!(
        concat!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}},",
            "\"finish_reason\":\"stop\"}}]}}\n\n"
        ),
        text = text.replace('"', "'")
    )
}

#[cfg(unix)]
pub(crate) fn harmless_body() -> String {
    "data: {\"content\":\"{\\\"proposals\\\": []}\"}\n\n".to_owned()
}
