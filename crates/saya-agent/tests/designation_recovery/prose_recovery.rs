/// A provider that replays a scripted stream per call, records every request,
/// and counts how many times it was called.
struct Scripted {
    turns: Vec<Vec<Result<ProviderEvent, ProviderError>>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl Scripted {
    fn turn(&self, index: usize) -> Vec<Result<ProviderEvent, ProviderError>> {
        self.turns
            .get(index)
            .unwrap_or_else(|| panic!("scripted turn {index} was not provided"))
            .clone()
    }
}

#[async_trait]
impl ChatProvider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn complete(&self, _: ChatRequest) -> Result<saya_agent::ChatResponse, ProviderError> {
        unreachable!("designation recovery drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let index = self.requests.lock().unwrap().len() - 1;
        Ok(Box::pin(futures_util::stream::iter(self.turn(index))))
    }
}

struct ExecutedTools {
    names: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for ExecutedTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.names.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"columns": ["a"], "rows": [[1]]}))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "show data".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "bounded_sql_query".into(),
        description: "read-only query".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: true,
            external_side_effect: false,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }]
}

fn designation_call(id: &str, sql: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "designate_answer".into(),
        arguments: serde_json::json!({"sql": sql}),
    }
}

fn text_then_done(text: &str) -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![
        Ok(ProviderEvent::TextDelta(text.into())),
        Ok(ProviderEvent::Done),
    ]
}

fn calls_only(calls: Vec<ToolCall>) -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![Ok(ProviderEvent::ToolCalls(calls)), Ok(ProviderEvent::Done)]
}

fn designated_events(output: &saya_agent::AgentOutput) -> Vec<&str> {
    output
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::AnswerDesignated { sql } => Some(sql.as_str()),
            _ => None,
        })
        .collect()
}

fn answered_call_ids(request: &ChatRequest) -> Vec<String> {
    request
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .filter_map(|message| message.tool_call_id.clone())
        .collect()
}

/// The measured failure: the model sends `designate_answer` with no prose. The
/// run must not end there — the designated SQL is recorded and answered, and
/// one follow-up turn produces the sentence around it.
#[tokio::test]
async fn empty_designation_takes_one_follow_up_turn_for_the_prose() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            text_then_done("The answer is 42."),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("an empty designation recovers its prose in one follow-up turn");
    assert_eq!(
        output.answer, "The answer is 42.",
        "an empty-prose designation must recover the prose in one bounded follow-up turn"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the designated SQL is carried on the output"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        2,
        "exactly one follow-up call: the designation turn plus the recovery turn"
    );
    assert_eq!(
        designated_events(&output),
        vec!["SELECT count(*) FROM t"],
        "exactly one AnswerDesignated event, for the first designation"
    );
    let second = &requests.lock().unwrap()[1];
    assert!(
        second.messages.iter().any(|message| {
            message.role == "tool" && message.tool_call_id.as_deref() == Some("d1")
        }),
        "the follow-up request must answer the designation call: {second:?}"
    );
}

/// A designation that arrives alongside its prose is unchanged: the run ends
/// there and the provider is not called again. (Pins existing behaviour.)
#[tokio::test]
async fn designation_with_prose_makes_no_extra_call() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")])
                .into_iter()
                .chain(std::iter::once(Ok(ProviderEvent::TextDelta(
                    "The answer is 42.".into(),
                ))))
                .collect(),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("a designation with prose completes the run");
    assert_eq!(output.answer, "The answer is 42.");
    assert_eq!(
        requests.lock().unwrap().len(),
        1,
        "a designation with prose must end the run without an extra provider call"
    );
}
