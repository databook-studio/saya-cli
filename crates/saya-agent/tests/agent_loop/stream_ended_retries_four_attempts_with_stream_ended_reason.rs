/// A provider whose every attempt reports that the stream ended before its
/// terminator without having delivered anything. A stream end *after* content
/// is a stream cut — a deterministic gateway time limit — and takes the
/// continuation path instead, which `a_cut_after_content_is_typed_and_not_resent`
/// pins.
struct StreamEndedProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}
#[async_trait]
impl ChatProvider for StreamEndedProvider {
    fn name(&self) -> &str {
        "stream-ended"
    }
    fn retry_delays(&self) -> Vec<std::time::Duration> {
        vec![std::time::Duration::from_millis(1); 8]
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
        Ok(Box::pin(futures_util::stream::iter(vec![Err(
            saya_agent::ProviderError::StreamEnded,
        )])))
    }
}
#[tokio::test]
async fn stream_ended_retries_nine_attempts_with_stream_ended_reason() {
    use saya_agent::{ProviderRecoveryPhase as Phase, ProviderRecoveryReason as Reason};
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = StreamEndedProvider {
        requests: requests.clone(),
    };
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = RecordingSink {
        events: events.clone(),
    };
    let error = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            AgentError::Provider(saya_agent::ProviderError::StreamEnded)
        ),
        "the exhausted run keeps the typed cause: {error:?}"
    );

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 9, "one attempt plus the eight retries");
    assert!(
        captured
            .iter()
            .all(|attempt| attempt.messages == captured[0].messages),
        "every attempt re-sends the turn-start conversation unchanged"
    );
    assert!(
        !captured.iter().any(|attempt| attempt
            .messages
            .iter()
            .any(|message| message.content.contains("partial"))),
        "the partial text never enters a request"
    );

    let seen = events.lock().unwrap().clone();
    let recoveries: Vec<_> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ProviderRecovery {
                phase,
                reason,
                attempt,
                limit,
            } => Some((*phase, *reason, *attempt, *limit)),
            _ => None,
        })
        .collect();
    assert_eq!(
        recoveries,
        vec![
            (Phase::Retrying, Reason::StreamEnded, 2, 9),
            (Phase::Retrying, Reason::StreamEnded, 3, 9),
            (Phase::Retrying, Reason::StreamEnded, 4, 9),
            (Phase::Retrying, Reason::StreamEnded, 5, 9),
            (Phase::Retrying, Reason::StreamEnded, 6, 9),
            (Phase::Retrying, Reason::StreamEnded, 7, 9),
            (Phase::Retrying, Reason::StreamEnded, 8, 9),
            (Phase::Retrying, Reason::StreamEnded, 9, 9),
            (Phase::Exhausted, Reason::StreamEnded, 9, 9),
        ],
        "every retry and the exhaustion carry the stream-ended reason: {seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|event| matches!(event, AgentEvent::TurnReset))
            .count(),
        8,
        "one reset per retry"
    );
}
