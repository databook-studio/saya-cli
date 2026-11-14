// A prompt cache is a prefix cache: every request's messages and tool list must
// begin with the previous request's, byte for byte. A scripted multi-turn job
// that reports reasoning each turn is the shape that used to break: attaching a
// new turn's reasoning cleared every earlier turn's copy, rewriting the prefix
// at the first assistant message and invalidating the cache for every token
// after it.

/// Answers the first `turns` calls with one tool call each — every one carrying
/// reasoning — and the last with a plain answer. Every request is recorded so
/// consecutive bodies can be compared.
struct ScriptedReasoningProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    turns: usize,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChatProvider for ScriptedReasoningProvider {
    fn name(&self) -> &str {
        "scripted-reasoning"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let turn = self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        let mut response = if turn < self.turns {
            saya_agent::ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: format!("call-{turn}"),
                    name: "schema_discovery".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
                images: Vec::new(),
                reasoning: None,
                reasoning_signature: None,
            })
        } else {
            saya_agent::ChatResponse::new(ChatMessage::text("assistant", "done"))
        };
        response.reasoning = Some(format!("turn {turn} reasoning"));
        Ok(response)
    }
}

#[tokio::test]
async fn every_request_body_extends_the_previous_one() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = ScriptedReasoningProvider {
        requests: requests.clone(),
        turns: 3,
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let tools = MockTools {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let output = run_agent(
        &provider,
        &tools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the run completes");
    assert_eq!(output.answer, "done");

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 4, "three tool turns and one answer");
    for (index, pair) in captured.windows(2).enumerate() {
        let (previous, next) = (&pair[0], &pair[1]);
        assert!(
            previous.messages.len() < next.messages.len(),
            "request {index} grows the history"
        );
        assert_eq!(
            previous.messages.as_slice(),
            &next.messages[..previous.messages.len()],
            "request {} begins with request {index}'s messages byte for byte",
            index + 1
        );
        assert_eq!(
            serde_json::to_vec(&previous.tools).unwrap(),
            serde_json::to_vec(&next.tools[..previous.tools.len()]).unwrap(),
            "request {} begins with request {index}'s tool list",
            index + 1
        );
    }
    let reasonings: Vec<_> = captured
        .last()
        .unwrap()
        .messages
        .iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.reasoning.as_deref())
        .collect();
    assert_eq!(
        reasonings,
        vec![
            Some("turn 0 reasoning"),
            Some("turn 1 reasoning"),
            Some("turn 2 reasoning"),
        ],
        "every earlier turn's reasoning stays on the wire"
    );
}
