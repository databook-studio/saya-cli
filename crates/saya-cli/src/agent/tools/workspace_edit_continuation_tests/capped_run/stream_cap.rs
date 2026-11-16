use super::*;

/// `MAX_STREAM_BYTES` still bounds a stream on the continuation path: a text
/// delta over the cap fails the turn with the size-limit error before any
/// continuation note is pushed, so the resume path cannot smuggle an
/// unbounded stream past the cap.
#[tokio::test]
async fn the_continuation_path_does_not_bypass_the_stream_cap() {
    struct CappedProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }
    #[async_trait::async_trait]
    impl ChatProvider for CappedProvider {
        fn name(&self) -> &str {
            "oversized-stream"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            unreachable!("the main loop drives the provider through stream")
        }
        async fn stream(
            &self,
            request: ChatRequest,
            _: CancellationToken,
        ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(saya_agent::ProviderEvent::TextDelta(over_stream_cap_text())),
                Ok(saya_agent::ProviderEvent::Done),
            ])))
        }
    }
    let sandbox = Sandbox::new("stream-cap");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CappedProvider {
        requests: requests.clone(),
    };
    let error = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_edit_definition()],
        limits(),
        &AllowReadOnlyApproval,
        &NoopSink,
        CancellationToken::new(),
    )
    .await
    .expect_err("an oversized stream must fail the run");
    assert!(
        error.to_string().contains("size limit"),
        "the stream cap must fire, got: {error}"
    );
    assert!(
        !sandbox.ws_root().join("report.txt").exists(),
        "the oversized turn ran no tool: no file may exist"
    );
    for seen in requests.lock().unwrap().iter() {
        assert_eq!(
            continuation_notes(&seen.messages),
            0,
            "no continuation note may be pushed for a stream-cap failure"
        );
    }
}
