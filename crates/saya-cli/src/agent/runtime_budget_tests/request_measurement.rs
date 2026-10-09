struct TimedSink {
    started: Instant,
    events: Mutex<Vec<AgentEvent>>,
    first_assistant_text: Mutex<Option<Duration>>,
}

impl TimedSink {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            events: Mutex::new(Vec::new()),
            first_assistant_text: Mutex::new(None),
        }
    }

    fn first_assistant_text(&self) -> Option<Duration> {
        *self.first_assistant_text.lock().unwrap()
    }

    fn event_count(&self, matches: fn(&AgentEvent) -> bool) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches(event))
            .count()
    }
}

#[async_trait]
impl AgentEventSink for TimedSink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(&event, AgentEvent::AssistantText { text } if !text.is_empty()) {
            let mut first = self.first_assistant_text.lock().unwrap();
            if first.is_none() {
                *first = Some(self.started.elapsed());
            }
        }
        self.events.lock().unwrap().push(event);
    }
}

struct RequestMeasurement {
    request: ChatRequest,
    request_count: usize,
    tool_calls: usize,
    retries: usize,
    elapsed: Duration,
    first_assistant_text: Option<Duration>,
}

async fn capture_repeated_requests() -> Vec<RequestMeasurement> {
    let mut measurements = Vec::new();
    for _ in 0..5 {
        let sink = TimedSink::new();
        let (result, counters) = run_turn_with_request(
            16 * 1024,
            "orders by month",
            Vec::new(),
            Some("SELECT id FROM orders"),
            test_ai(16 * 1024),
            "analytics",
            &sink,
        )
        .await;
        result.expect("the fixed local turn completes");
        measurements.push(RequestMeasurement {
            request: counters.first_request().expect("a request was captured"),
            request_count: counters.request_count(),
            tool_calls: sink.event_count(|event| matches!(event, AgentEvent::ToolRequested { .. })),
            retries: sink.event_count(|event| matches!(event, AgentEvent::TurnReset)),
            elapsed: sink.started.elapsed(),
            first_assistant_text: sink.first_assistant_text(),
        });
    }
    measurements
}
