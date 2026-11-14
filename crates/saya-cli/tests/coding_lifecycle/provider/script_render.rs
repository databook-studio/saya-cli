//! SSE encoding shared by the independent fixture scenarios.
use serde_json::{Value, json};

pub(crate) fn tool(name: &str, args: Value) -> String {
    tagged_tool(&format!("call-{name}"), name, args)
}
pub(crate) fn tagged_tool(id: &str, name: &str, args: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
pub(crate) fn tools(calls: &[(&str, &str, Value)]) -> String {
    let calls = calls
        .iter()
        .enumerate()
        .map(|(index, (id, name, args))| json!({"index":index,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}))
        .collect::<Vec<_>>();
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":calls},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
/// One provider usage report, with the optional cache and reasoning details
/// only when the scenario says the provider reported them.
pub(crate) fn usage(input: u64, output: u64, cached: Option<u64>, reasoning: Option<u64>) -> Value {
    let mut usage = json!({"prompt_tokens":input,"completion_tokens":output});
    if let Some(cached) = cached {
        usage["prompt_tokens_details"] = json!({"cached_tokens":cached});
    }
    if let Some(reasoning) = reasoning {
        usage["completion_tokens_details"] = json!({"reasoning_tokens":reasoning});
    }
    usage
}
pub(crate) fn usage_tool(id: &str, name: &str, args: Value, usage: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":usage})
    )
}
pub(crate) fn usage_answer(text: &str, usage: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":usage})
    )
}
pub(crate) fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
