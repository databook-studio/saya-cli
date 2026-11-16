// Through the loop: the first request carries the operator's
// configured ceiling verbatim, and the adaptive policy's first-request effort
// reaches the Anthropic route as adaptive thinking.

struct NoTools;

#[async_trait::async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        unreachable!("a run with no tool definitions never executes a tool")
    }
}

fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "hello".into(),
        profile_names: vec!["analytics".into()],
        model: "test-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// A configured 131,072-token ceiling reaches the first request unchanged —
/// there is no first-request fit that lowers it.
#[tokio::test]
async fn the_first_request_sends_the_configured_output_ceiling() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n"],
    }]);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO])
            .with_max_output_tokens(Some(131_072)),
        Some("k"),
    )
    .unwrap();
    run_agent(
        &provider,
        &NoTools,
        agent_request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a one-turn answer completes");
    handle.join().unwrap();

    let sent = requests.lock().unwrap()[0].clone();
    assert!(
        sent.contains("\"max_completion_tokens\":131072"),
        "the first request must send the configured ceiling verbatim: {sent}"
    );
}

/// The adaptive policy's first request asks for high effort, and the
/// Anthropic route spells that as adaptive thinking plus `output_config`.
/// (Before adaptive-by-default this test asserted the opposite: an unset
/// effort sent nothing. That behaviour is intentionally changed.)
#[tokio::test]
async fn the_anthropic_route_spells_the_loops_first_request_effort() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider = saya_agent::AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("k"),
    )
    .unwrap();
    run_agent(
        &provider,
        &NoTools,
        agent_request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a one-turn answer completes");
    handle.join().unwrap();

    let sent = requests.lock().unwrap()[0].clone();
    assert!(
        sent.contains("\"thinking\":{\"type\":\"adaptive\"}"),
        "the first request of a run thinks hard: {sent}"
    );
    assert!(
        sent.contains("\"output_config\":{\"effort\":\"high\"}"),
        "the first request asks for high effort: {sent}"
    );
}
