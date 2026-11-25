//! The history policy's budget: a long, scripted job must not re-send old tool
//! results in full on every request.
//!
//! The workload is fixed: [`TOOL_TURNS`] turns of realistic read, command and
//! search results, then a final answer. It drives the real agent loop with a
//! scripted provider, so the request bytes are the bytes the loop would send.
//! The same workload's untrimmed request is reconstructed from the recorded
//! requests by restoring the full tool results, which gives the baseline the
//! assertions compare against. The test asserts behaviour — below-budget
//! requests are byte-identical, every read stays whole, old non-read output is
//! stubbed, the newest groups stay whole — and that the mean request is
//! strictly smaller. It does not pin a ratio: a ratio would reward stubbing
//! results the model still needs.

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, LocalStateEffect, ProviderError, ToolCall, ToolConcurrency, ToolDefinition,
    ToolEffect, ToolError, ToolExecutor, run_agent,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Tool-result turns the scripted job runs before it answers.
const TOOL_TURNS: usize = 22;
/// Provider requests the job makes: one per tool turn plus the answer.
const REQUESTS: usize = TOOL_TURNS + 1;
/// Leading sentinel the history policy writes on a stubbed result.
const STUB_MARKER: &str = "…[history-stub:";
/// The bytes-per-token estimate the loop uses for its own accounting.
const BYTES_PER_TOKEN: f64 = 3.5;
/// The newest groups whose results the policy keeps whole.
const LAST_WHOLE_GROUPS: usize = 6;

fn workload_call(index: usize) -> (String, serde_json::Value) {
    match index % 3 {
        0 => (
            "read".to_owned(),
            json!({"path": format!("src/module_{index}.rs"), "start_line": 1, "line_count": 200}),
        ),
        1 => (
            "shell".to_owned(),
            json!({"command": format!("cargo test --package module_{index}")}),
        ),
        _ => (
            "grep".to_owned(),
            json!({"pattern": format!("handle_request_{index}"), "path": "src"}),
        ),
    }
}

/// The full result a tool returns for a call — deterministic, so the
/// untrimmed baseline can be reconstructed from the call alone.
fn tool_output(name: &str, arguments: &serde_json::Value) -> serde_json::Value {
    match name {
        "read" => {
            let path = arguments["path"].as_str().unwrap_or("src/lib.rs");
            let content = (1..=500)
                .map(|line| format!("{line:03}: pub fn item_{line}() -> usize {{ {line} }}"))
                .collect::<Vec<_>>()
                .join("\n");
            json!({
                "path": path,
                "start_line": 1,
                "lines_returned": 500,
                "content": content,
            })
        }
        "shell" => {
            let command = arguments["command"].as_str().unwrap_or("cargo test");
            let stdout = (1..=450)
                .map(|line| format!("test module::case_{line:03} ... ok"))
                .collect::<Vec<_>>()
                .join("\n");
            json!({"command": command, "exit_code": 0, "stdout": stdout})
        }
        _ => {
            let pattern = arguments["pattern"].as_str().unwrap_or("TODO");
            let matches = (1..=200)
                .map(|index| {
                    json!({
                        "path": format!("src/module_{index}.rs"),
                        "line": index * 7,
                        "text": format!("{pattern} appears in module {index} at line {}", index * 7),
                    })
                })
                .collect::<Vec<_>>();
            json!({"pattern": pattern, "matches": matches})
        }
    }
}

struct ScriptedJob {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    turn: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for ScriptedJob {
    fn name(&self) -> &str {
        "scripted-history-workload"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut turn = self.turn.lock().unwrap();
        let index = *turn;
        *turn += 1;
        if index < TOOL_TURNS {
            let (name, arguments) = workload_call(index);
            return Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: format!("turn {index}: inspecting"),
                tool_calls: vec![ToolCall {
                    id: format!("call-{index}"),
                    name,
                    arguments,
                }],
                ..Default::default()
            }));
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

struct WorkloadTools;

#[async_trait]
impl ToolExecutor for WorkloadTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        Ok(tool_output(name, &arguments))
    }
}

fn definitions() -> Vec<ToolDefinition> {
    ["read", "shell", "grep"]
        .into_iter()
        .map(|name| ToolDefinition {
            name: name.into(),
            description: format!("{name} fixture"),
            read_only: true,
            parameters: json!({"type": "object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        })
        .collect()
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "inspect the workspace and report".into(),
        profile_names: Vec::new(),
        model: "scripted-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn limits() -> AgentLimits {
    AgentLimits {
        max_turns: Some(40),
        max_tool_calls: Some(64),
        max_continuations: Some(3),
        context_byte_budget: 8 * 1024 * 1024,
        ..Default::default()
    }
}

fn body_bytes(request: &ChatRequest) -> usize {
    serde_json::to_vec(request)
        .expect("request serializes")
        .len()
}

/// The same request with every stub replaced by the full result it points at.
fn untrimmed(request: &ChatRequest) -> ChatRequest {
    let calls: HashMap<&str, (&str, &serde_json::Value)> = request
        .messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .map(|call| (call.id.as_str(), (call.name.as_str(), &call.arguments)))
        .collect();
    let mut full = request.clone();
    for message in &mut full.messages {
        if message.role != "tool" || !message.content.starts_with(STUB_MARKER) {
            continue;
        }
        let Some((name, arguments)) = message
            .tool_call_id
            .as_deref()
            .and_then(|id| calls.get(id).copied())
        else {
            continue;
        };
        message.content =
            serde_json::to_string(&tool_output(name, arguments)).expect("result serializes");
    }
    full
}

fn mean(values: &[usize]) -> usize {
    values.iter().sum::<usize>() / values.len()
}

#[tokio::test]
async fn a_long_job_stubs_only_old_non_read_output_and_keeps_reads_whole() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedJob {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &WorkloadTools,
        request(),
        definitions(),
        limits(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the scripted job completes");
    assert_eq!(output.answer, "done");
    let requests = requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        REQUESTS,
        "the scripted turn count is unchanged"
    );
    let actual = requests.iter().map(body_bytes).collect::<Vec<_>>();
    let baseline = requests
        .iter()
        .map(|request| body_bytes(&untrimmed(request)))
        .collect::<Vec<_>>();
    let actual_mean = mean(&actual);
    let baseline_mean = mean(&baseline);
    let token_estimate = (actual_mean as f64 / BYTES_PER_TOKEN).ceil() as usize;
    println!(
        "history workload: {} requests, mean {} B (untrimmed {} B, ~{} tokens)",
        requests.len(),
        actual_mean,
        baseline_mean,
        token_estimate
    );
    println!("request bytes: {actual:?}");
    println!("untrimmed bytes: {baseline:?}");
    assert!(
        actual_mean < baseline_mean,
        "a history over its budget must send fewer bytes: {actual_mean} vs {baseline_mean}"
    );
    assert_eq!(
        actual[0], baseline[0],
        "a first request below the budget is byte-identical"
    );
    assert_eq!(
        &actual[..10],
        &baseline[..10],
        "a short job below the budget sends exactly what it sent before the policy"
    );
    let last = requests.last().expect("at least one request");
    // Every read is the latest read of its path in this workload, so every
    // read result stays whole. Stubbing one would make the model re-read it.
    let read_ids = last
        .messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .filter(|call| call.name == "read")
        .map(|call| call.id.as_str())
        .collect::<Vec<_>>();
    for message in &last.messages {
        let is_read = message
            .tool_call_id
            .as_deref()
            .is_some_and(|id| read_ids.contains(&id));
        if is_read {
            assert!(
                !message.content.starts_with(STUB_MARKER),
                "a read result is the model's working copy: {}",
                message.content
            );
        }
    }
    let tool_messages = last
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .collect::<Vec<_>>();
    let stubs = tool_messages
        .iter()
        .filter(|message| message.content.starts_with(STUB_MARKER))
        .count();
    assert!(stubs > 0, "old non-read output is stubbed over the budget");
    for message in &tool_messages[tool_messages.len() - LAST_WHOLE_GROUPS..] {
        assert!(
            !message.content.starts_with(STUB_MARKER),
            "the newest groups stay whole: {}",
            message.content
        );
    }
    let stub = tool_messages
        .iter()
        .find(|message| message.content.starts_with(STUB_MARKER))
        .expect("a stub is present");
    assert!(
        stub.content
            .contains("re-read: call shell again with the same arguments"),
        "the model sees how to re-read an old non-read result: {}",
        stub.content
    );
}
