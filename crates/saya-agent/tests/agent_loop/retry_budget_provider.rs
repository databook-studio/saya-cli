/// What one attempt of the scripted provider does. `Retryable` is a
/// content-free mid-stream failure, the loop's retryable reading; `Hang` is a
/// stream that never yields, so only a deadline can end it.
#[derive(Clone, Copy)]
enum RetryStep {
    Retryable,
    Answer,
    Hang,
}

struct RetryBudgetProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    delays: Vec<Duration>,
    steps: Mutex<VecDeque<RetryStep>>,
}

impl RetryBudgetProvider {
    fn new(delays: Vec<Duration>, steps: Vec<RetryStep>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            delays,
            steps: Mutex::new(steps.into()),
        }
    }
}

#[async_trait]
impl ChatProvider for RetryBudgetProvider {
    fn name(&self) -> &str {
        "retry-budget"
    }
    fn retry_delays(&self) -> Vec<Duration> {
        self.delays.clone()
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(RetryStep::Answer);
        let events: Vec<Result<ProviderEvent, saya_agent::ProviderError>> = match step {
            RetryStep::Retryable => vec![Err(saya_agent::ProviderError::StreamEnded)],
            RetryStep::Answer => vec![
                Ok(ProviderEvent::TextDelta("answer".into())),
                Ok(ProviderEvent::Done),
            ],
            RetryStep::Hang => {
                return Ok(Box::pin(futures_util::stream::pending()));
            }
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Runs one controlled (answering-budget) job under the given wall clock.
async fn run_capped(
    provider: &dyn ChatProvider,
    wall_clock: Option<Duration>,
) -> Result<saya_agent::AgentOutput, AgentError> {
    let limits = AgentLimits {
        progress: Some(saya_agent::ProgressLimits {
            wall_clock,
            ..saya_agent::ProgressLimits::default()
        }),
        ..AgentLimits::default()
    };
    let approval = AllowReadOnlyApproval;
    let sink = saya_agent::NoopEventSink;
    let control = saya_agent::AgentRunControl::new(
        limits,
        saya_agent::AnsweringLimits {
            max_requests: Some(8),
            max_tool_calls: Some(8),
            max_reported_tokens: Some(10_000),
        },
        &approval,
        &sink,
        CancellationToken::new(),
    );
    let tools = MockTools {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    saya_agent::run_agent_with_control(provider, &tools, request(), definitions(), control).await
}

fn stopped_on_the_clock(output: &saya_agent::AgentOutput) -> bool {
    output.events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: saya_agent::BudgetDimension::ElapsedTime
            }
        )
    })
}
