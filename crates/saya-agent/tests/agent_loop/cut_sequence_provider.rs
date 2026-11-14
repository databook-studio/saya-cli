/// A provider that cuts its first `cuts` attempts after delivering reasoning,
/// then answers. A cut that delivered content must not make the next request
/// the run's first request again.
struct CutThenAnswerProvider {
    cuts: usize,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for CutThenAnswerProvider {
    fn name(&self) -> &str {
        "cut-then-answer"
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
        let turn = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls
        };
        if turn <= self.cuts {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::ReasoningDelta("x".repeat(350))),
                Err(saya_agent::ProviderError::StreamEnded),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::TextDelta("final answer".into())),
            Ok(ProviderEvent::Done),
        ])))
    }
}

/// A provider that cuts on even calls and returns a read-only tool call on odd
/// calls until `answer_at`, then answers. Every cut is separated by a
/// completed reply, so the cuts are not consecutive and must not end the run.
struct AlternatingCutProvider {
    answer_at: usize,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    calls: Mutex<usize>,
}

#[async_trait]
impl ChatProvider for AlternatingCutProvider {
    fn name(&self) -> &str {
        "alternating-cut"
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
        let turn = {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            *calls - 1
        };
        if turn >= self.answer_at {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("final answer".into())),
                Ok(ProviderEvent::Done),
            ])));
        }
        if turn % 2 == 0 {
            return Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::ReasoningDelta("x".repeat(350))),
                Err(saya_agent::ProviderError::StreamEnded),
            ])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                id: format!("call-{turn}"),
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({"sql": "SELECT 1"}),
            }])),
            Ok(ProviderEvent::Done),
        ])))
    }
}

/// A provider whose establishment succeeds but whose stream never yields. The
/// request deadline must bound the open stream, and the call counter proves
/// the request was opened.
struct HangingStreamProvider {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl ChatProvider for HangingStreamProvider {
    fn name(&self) -> &str {
        "hanging-stream"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

/// A provider whose establishment never completes. The loop's own deadline
/// must bound it; the call counter proves whether the request was opened at
/// all.
struct HangingEstablishmentProvider {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl ChatProvider for HangingEstablishmentProvider {
    fn name(&self) -> &str {
        "hanging-establishment"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!("the main loop drives the provider through stream")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::pending().await
    }
}
