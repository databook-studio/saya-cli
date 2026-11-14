// The adaptive policy's per-turn effort, as it reaches the Anthropic route.
//
// `low` is the routine-turn arm. It must go out the way the provider documents
// its own effort control — adaptive thinking plus `output_config.effort` — not
// as a fixed thinking budget, which is what every routine turn used to send.
// `high` (the first request and any request after a failed tool result) stays
// adaptive. The OpenAI route keeps its `reasoning_effort` spelling.

const ANTHROPIC_TOOL_USE: &str = concat!(
    "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,",
    "\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-1\",\"name\":\"schema_discovery\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,",
    "\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
);

const ANTHROPIC_TEXT: &str = concat!(
    "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,",
    "\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
);

const OPENAI_TOOL_USE: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",",
    "\"function\":{\"name\":\"schema_discovery\",\"arguments\":\"{}\"}}]}}]}\n\n",
    "data: [DONE]\n\n",
);

const OPENAI_TEXT: &str =
    "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n";

struct SucceedingSchemaTool;

#[async_trait::async_trait]
impl ToolExecutor for SucceedingSchemaTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Ok(serde_json::json!({"tables": []}))
    }
}

struct FailingSchemaTool;

#[async_trait::async_trait]
impl ToolExecutor for FailingSchemaTool {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        Err(ToolError::QueryFailedDetail("boom".into()))
    }
}

fn policy_agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "test-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn discovery_tool() -> ToolDefinition {
    ToolDefinition {
        name: "schema_discovery".into(),
        description: "schema discovery".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}

fn sent_body(captured: &str) -> serde_json::Value {
    serde_json::from_str(
        captured
            .split("\r\n\r\n")
            .nth(1)
            .expect("the captured request carries a body"),
    )
    .expect("the request body is JSON")
}

async fn anthropic_policy_trial(tools: &dyn ToolExecutor) -> Vec<serde_json::Value> {
    let (base, requests, handle) = server(vec![
        Reply {
            status: 200,
            chunks: vec![ANTHROPIC_TOOL_USE],
        },
        Reply {
            status: 200,
            chunks: vec![ANTHROPIC_TEXT],
        },
    ]);
    let provider = saya_agent::AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();
    run_agent(
        &provider,
        tools,
        policy_agent_request(),
        vec![discovery_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a tool turn then an answer completes");
    handle.join().unwrap();
    requests
        .lock()
        .unwrap()
        .iter()
        .map(|captured| sent_body(captured))
        .collect()
}

/// The first request thinks hard; the routine follow-up after a clean tool
/// result thinks lightly, both as adaptive thinking with an effort setting.
#[tokio::test]
async fn the_adaptive_policy_spells_routine_turns_as_adaptive_low() {
    let bodies = anthropic_policy_trial(&SucceedingSchemaTool).await;
    assert_eq!(bodies.len(), 2, "one tool turn, one answer turn");
    assert_eq!(
        bodies[0]["thinking"],
        serde_json::json!({"type": "adaptive"})
    );
    assert_eq!(
        bodies[0]["output_config"],
        serde_json::json!({"effort": "high"})
    );
    assert_eq!(
        bodies[1]["thinking"],
        serde_json::json!({"type": "adaptive"}),
        "a routine turn must not fall back to a fixed thinking budget: {}",
        bodies[1]
    );
    assert_eq!(
        bodies[1]["output_config"],
        serde_json::json!({"effort": "low"}),
        "a routine turn asks for low effort: {}",
        bodies[1]
    );
    assert!(
        bodies[1].get("temperature").is_none(),
        "adaptive thinking rejects temperature: {}",
        bodies[1]
    );
}

/// A failed tool result is the policy's "think hard" arm: the next request is
/// adaptive with high effort, not the routine low.
#[tokio::test]
async fn a_failed_tool_result_returns_to_adaptive_high() {
    let bodies = anthropic_policy_trial(&FailingSchemaTool).await;
    assert_eq!(bodies.len(), 2, "one failed tool turn, one answer turn");
    assert_eq!(
        bodies[1]["thinking"],
        serde_json::json!({"type": "adaptive"})
    );
    assert_eq!(
        bodies[1]["output_config"],
        serde_json::json!({"effort": "high"})
    );
}

/// The OpenAI-compatible route keeps the flat `reasoning_effort` spelling for
/// the same adaptive policy; the Anthropic change must not reach it.
#[tokio::test]
async fn the_openai_route_keeps_reasoning_effort_low_on_routine_turns() {
    let (base, requests, handle) = server(vec![
        Reply {
            status: 200,
            chunks: vec![OPENAI_TOOL_USE],
        },
        Reply {
            status: 200,
            chunks: vec![OPENAI_TEXT],
        },
    ]);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();
    run_agent(
        &provider,
        &SucceedingSchemaTool,
        policy_agent_request(),
        vec![discovery_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a tool turn then an answer completes");
    handle.join().unwrap();
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "one tool turn, one answer turn");
    let body = sent_body(&captured[1]);
    assert_eq!(
        body["reasoning_effort"],
        serde_json::json!("low"),
        "the OpenAI route still spells routine effort low: {body}"
    );
    assert!(
        body.get("thinking").is_none() && body.get("output_config").is_none(),
        "the Anthropic thinking shape must not reach the OpenAI body: {body}"
    );
}
