/// The messages of the first provider call: the history the loop assembled
/// under the config `context_byte_budget`.
#[derive(Default)]
struct Counters {
    first_answering_request: Mutex<Option<ChatRequest>>,
    request_count: Mutex<usize>,
}

impl Counters {
    fn first_messages(&self) -> Option<Vec<ChatMessage>> {
        self.first_request().map(|request| request.messages)
    }

    fn first_request(&self) -> Option<ChatRequest> {
        self.first_answering_request.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        *self.request_count.lock().unwrap()
    }
}

/// A provider that answers every call with plain text, recording the first
/// request's messages.
struct AnswerProbe {
    counters: Arc<Counters>,
}

#[async_trait]
impl ChatProvider for AnswerProbe {
    fn name(&self) -> &str {
        "budget-probe"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        *self.counters.request_count.lock().unwrap() += 1;
        {
            let mut first = self.counters.first_answering_request.lock().unwrap();
            if first.is_none() {
                *first = Some(request.clone());
            }
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

/// Runs one turn through the real runtime path, returning the result and the
/// probe counters.
async fn run_turn(
    context_byte_budget: usize,
    history: Vec<ChatMessage>,
) -> (Result<AgentOutput, AgentRuntimeError>, Arc<Counters>) {
    let sink = NoopEventSink;
    run_turn_with_request(
        context_byte_budget,
        "orders by month",
        history,
        None,
        test_ai(context_byte_budget),
        "analytics",
        &sink,
    )
    .await
}

async fn run_turn_with_request(
    context_byte_budget: usize,
    prompt: &str,
    history: Vec<ChatMessage>,
    last_sql: Option<&str>,
    ai: ResolvedAi,
    profile: &str,
    sink: &dyn AgentEventSink,
) -> (Result<AgentOutput, AgentRuntimeError>, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let inputs = TurnInputs {
        ai,
        provider: Box::new(AnswerProbe {
            counters: Arc::clone(&counters),
        }),
        registry: registry_for(profile),
        failures: Vec::new(),
    };
    let runtime = test_runtime(context_byte_budget);
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        prompt,
        ApprovalPolicy::ReadOnly,
        false,
        false,
        history,
        sink,
        CancellationToken::new(),
        None,
        None,
        last_sql.map(str::to_owned),
        None,
        AgentMode::Build,
        None,
        None,
    )
    .await;
    (result, counters)
}

async fn record_first_request(prompt: &str, last_sql: Option<&str>) -> ChatRequest {
    record_first_request_for(
        prompt,
        last_sql,
        test_ai(16 * 1024),
        "analytics",
        &NoopEventSink,
    )
    .await
}

async fn record_first_request_for(
    prompt: &str,
    last_sql: Option<&str>,
    ai: ResolvedAi,
    profile: &str,
    sink: &dyn AgentEventSink,
) -> ChatRequest {
    let (result, counters) =
        run_turn_with_request(16 * 1024, prompt, Vec::new(), last_sql, ai, profile, sink).await;
    result.expect("the turn completes");
    counters
        .first_request()
        .expect("the provider receives the runtime's first complete request")
}
