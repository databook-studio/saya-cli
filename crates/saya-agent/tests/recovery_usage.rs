use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderRecoveryPhase, ProviderRecoveryReason, ProviderStream, TokenUsage, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

struct NoTools {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        *self.calls.lock().unwrap() += 1;
        Ok(serde_json::Value::Null)
    }
}

struct RecordingSink(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "answer".into(),
        profile_names: Vec::new(),
        model: "test".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

struct EstablishmentFailure {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for EstablishmentFailure {
    fn name(&self) -> &str {
        "establishment-failure"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        Err(ProviderError::Request("request refused".into()))
    }
}

/// An HTTP establishment failure has already had provider-layer retry policy.
/// The outer receiver must not replay an invalid request or execute effects.
#[tokio::test]
async fn stream_establishment_failure_is_not_retried() {
    let provider_calls = Arc::new(Mutex::new(0));
    let tool_calls = Arc::new(Mutex::new(0));
    let events = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &EstablishmentFailure {
            calls: provider_calls.clone(),
        },
        &NoTools {
            calls: tool_calls.clone(),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .expect_err("the refusal is terminal");
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::Request(_))
    ));
    assert_eq!(*provider_calls.lock().unwrap(), 1, "no outer receive retry");
    assert_eq!(*tool_calls.lock().unwrap(), 0, "no effects before a reply");
    let events = events.lock().unwrap();
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, AgentEvent::TurnReset)),
        "a non-retry has no reset: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::NotRetried,
                reason: ProviderRecoveryReason::ProviderFailure,
                attempt: 1,
                limit: 4,
            }
        )),
        "the receipt is typed but never derives a reason from provider prose: {events:?}"
    );
}

struct MaxThenSuccess {
    attempts: Mutex<usize>,
}

struct LatestUsageThenSuccess {
    attempts: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for LatestUsageThenSuccess {
    fn name(&self) -> &str {
        "latest-usage-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        let events = if *attempts == 1 {
            vec![
                Ok(ProviderEvent::TextDelta("discarded".into())),
                Ok(ProviderEvent::Usage(TokenUsage::new(3, 1))),
                Ok(ProviderEvent::Usage(TokenUsage::new(7, 2))),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Usage(TokenUsage::new(11, 5))),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// The failed receipt is the final cumulative snapshot, not a sum of stream
/// snapshots. Reset precedes the typed retry progress and all progress uses a
/// one-based actual-attempt count out of the four allowed calls.
#[tokio::test]
async fn retry_uses_latest_failed_snapshot_and_unambiguous_progress() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &LatestUsageThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .expect("the second attempt answers");
    assert_eq!(output.answer, "kept");
    assert_eq!(output.usage, TokenUsage::new(18, 7));
    let events = events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::FailedAttemptUsage { usage } => Some(*usage),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![TokenUsage::new(7, 2)]
    );
    let reset = events
        .iter()
        .position(|event| matches!(event, AgentEvent::TurnReset))
        .expect("a retry resets discarded text");
    assert!(matches!(
        events.get(reset + 1),
        Some(AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::Retrying,
            reason: ProviderRecoveryReason::StreamEnded,
            attempt: 2,
            limit: 4,
        })
    ));
}

struct DroppingProvider {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for DroppingProvider {
    fn name(&self) -> &str {
        "dropping"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.calls.lock().unwrap() += 1;
        Ok(Box::pin(futures_util::stream::empty()))
    }
}

struct CancelOnRetry(CancellationToken);

#[async_trait]
impl AgentEventSink for CancelOnRetry {
    async fn emit(&self, event: AgentEvent) {
        if matches!(
            event,
            AgentEvent::ProviderRecovery {
                phase: ProviderRecoveryPhase::Retrying,
                ..
            }
        ) {
            self.0.cancel();
        }
    }
}

#[tokio::test]
async fn cancellation_during_backoff_makes_no_next_provider_call() {
    let calls = Arc::new(Mutex::new(0));
    let cancellation = CancellationToken::new();
    let error = run_agent_with_sink(
        &DroppingProvider {
            calls: calls.clone(),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &CancelOnRetry(cancellation.clone()),
        cancellation,
    )
    .await
    .expect_err("cancellation abandons the local backoff");
    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(*calls.lock().unwrap(), 1);
}

struct ByteLimitThenSuccess {
    attempts: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for ByteLimitThenSuccess {
    fn name(&self) -> &str {
        "byte-limit-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        let events = if *attempts == 1 {
            vec![Ok(ProviderEvent::TextDelta(
                "x".repeat(saya_agent::MAX_STREAM_BYTES + 1),
            ))]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[tokio::test]
async fn byte_limit_has_a_typed_retry_reason() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let output = run_agent_with_sink(
        &ByteLimitThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink(events.clone()),
        CancellationToken::new(),
    )
    .await
    .expect("the retry answers");
    assert_eq!(output.answer, "kept");
    assert!(events.lock().unwrap().iter().any(|event| matches!(
        event,
        AgentEvent::ProviderRecovery {
            phase: ProviderRecoveryPhase::Retrying,
            reason: ProviderRecoveryReason::StreamByteLimit,
            attempt: 2,
            limit: 4,
        }
    )));
}

struct OptionalUsageThenSuccess {
    attempts: Mutex<usize>,
    first_usage: Option<TokenUsage>,
}

#[async_trait]
impl ChatProvider for OptionalUsageThenSuccess {
    fn name(&self) -> &str {
        "optional-usage-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let mut attempts = self.attempts.lock().unwrap();
        *attempts += 1;
        let events = if *attempts == 1 {
            self.first_usage
                .map(|usage| vec![Ok(ProviderEvent::Usage(usage))])
                .unwrap_or_default()
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("kept".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[tokio::test]
async fn unknown_failed_usage_is_absent_but_reported_zero_is_a_receipt() {
    for first_usage in [None, Some(TokenUsage::new(0, 0))] {
        let events = Arc::new(Mutex::new(Vec::new()));
        run_agent_with_sink(
            &OptionalUsageThenSuccess {
                attempts: Mutex::new(0),
                first_usage,
            },
            &NoTools {
                calls: Arc::new(Mutex::new(0)),
            },
            request(),
            Vec::new(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
            &RecordingSink(events.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("the retry answers");
        let receipt_count = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AgentEvent::FailedAttemptUsage { .. }))
            .count();
        assert_eq!(receipt_count, usize::from(first_usage.is_some()));
    }
}

#[async_trait]
impl ChatProvider for MaxThenSuccess {
    fn name(&self) -> &str {
        "max-then-success"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent receive path streams")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let attempt = {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            *attempts
        };
        let events = if attempt == 1 {
            vec![Ok(ProviderEvent::Usage(
                TokenUsage::new(u64::MAX, u64::MAX)
                    .with_cached_input(Some(u64::MAX))
                    .with_cache_creation(Some(u64::MAX))
                    .with_reasoning(Some(u64::MAX)),
            ))]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Usage(
                    TokenUsage::new(1, 1)
                        .with_cached_input(Some(1))
                        .with_cache_creation(Some(1))
                        .with_reasoning(Some(1)),
                )),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// Provider counts are untrusted numeric input: retries must saturate receipts
/// instead of panicking in debug builds or wrapping in release builds.
#[tokio::test]
async fn failed_and_successful_usage_saturates_at_u64_max() {
    let output = run_agent_with_sink(
        &MaxThenSuccess {
            attempts: Mutex::new(0),
        },
        &NoTools {
            calls: Arc::new(Mutex::new(0)),
        },
        request(),
        Vec::new(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .await
    .expect("the retry completes");
    assert_eq!(output.usage.input_tokens, u64::MAX);
    assert_eq!(output.usage.output_tokens, u64::MAX);
    assert_eq!(output.usage.cached_input_tokens, Some(u64::MAX));
    assert_eq!(output.usage.cache_creation_input_tokens, Some(u64::MAX));
    assert_eq!(output.usage.reasoning_tokens, Some(u64::MAX));
}
