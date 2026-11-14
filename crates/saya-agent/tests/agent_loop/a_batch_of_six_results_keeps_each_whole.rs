// A turn's tool results are capped per call, not out of a shared per-turn
// pool. Six calls in one batch each keep their whole 60 KiB result, even
// though the six together exceed a 256 KiB per-turn pool.

const CALLS: usize = 6;
const PAYLOAD: usize = 60 * 1024;

#[derive(Default)]
struct WideResultTools {
    caps: Mutex<Vec<usize>>,
}

#[async_trait]
impl ToolExecutor for WideResultTools {
    async fn execute(
        &self,
        _: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let which = arguments["which"].as_u64().expect("test supplies an index");
        Ok(serde_json::json!({
            "rows": [format!("{which}-{}", "x".repeat(PAYLOAD))]
        }))
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.caps.lock().unwrap().push(context.result_cap);
        self.execute(name, arguments).await
    }
}

struct SixCallProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    turn: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for SixCallProvider {
    fn name(&self) -> &str {
        "six-call"
    }

    async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut turn = self.turn.lock().unwrap();
        if *turn == 0 {
            *turn += 1;
            return Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: (0..CALLS)
                    .map(|which| ToolCall {
                        id: format!("call-{which}"),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({"which": which}),
                    })
                    .collect(),
                tool_call_id: None,
                images: Vec::new(),
                            reasoning: None,
                            reasoning_signature: None,
            }));
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

#[tokio::test]
async fn a_batch_of_six_results_keeps_each_whole() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let tools = WideResultTools::default();
    let output = run_agent(
        &SixCallProvider {
            requests: requests.clone(),
            turn: Mutex::new(0),
        },
        &tools,
        request(),
        definitions(),
        AgentLimits {
            context_byte_budget: 512 * 1024,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("bounded results complete");
    assert_eq!(output.answer, "done");
    assert_eq!(
        *tools.caps.lock().unwrap(),
        vec![64 * 1024; CALLS],
        "every call in the batch gets the per-call cap, with no per-turn split"
    );

    let captured = requests.lock().unwrap();
    let results: Vec<_> = captured[1]
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .collect();
    assert_eq!(results.len(), CALLS, "one result per accepted call");
    for (which, message) in results.iter().enumerate() {
        assert!(
            !message.content.contains("[truncated]"),
            "call {which} must keep its whole result: {} bytes",
            message.content.len()
        );
        assert!(
            message.content.len() > PAYLOAD,
            "call {which} kept only {} of {PAYLOAD} payload bytes",
            message.content.len()
        );
    }
}
