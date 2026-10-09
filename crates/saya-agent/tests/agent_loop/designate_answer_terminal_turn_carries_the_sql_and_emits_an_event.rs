/// The model designates the SQL that answers the question by calling
/// `designate_answer` in its terminal turn, alongside the prose answer. The
/// run ends there: the prose is the answer, the designated SQL is carried on
/// the output, and an `AnswerDesignated` event reaches the stream so a
/// headless reader can pair the answer with its query without guessing. A turn
/// that never designates works exactly as today (no SQL carried).
#[tokio::test]
async fn designate_answer_terminal_turn_carries_the_sql_and_emits_an_event() {
    struct DesignationProvider;
    #[async_trait]
    impl ChatProvider for DesignationProvider {
        fn name(&self) -> &str {
            "designate"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            unreachable!("designation drives the provider through stream")
        }
        async fn stream(
            &self,
            _: ChatRequest,
            _: CancellationToken,
        ) -> Result<ProviderStream, saya_agent::ProviderError> {
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(ProviderEvent::TextDelta("The answer is 42.".into())),
                Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                    id: "d1".into(),
                    name: "designate_answer".into(),
                    arguments: serde_json::json!({"sql": "SELECT count(*) FROM t"}),
                }])),
                Ok(ProviderEvent::Done),
            ])))
        }
    }
    let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
    struct RecordSink {
        events: Arc<Mutex<Vec<AgentEvent>>>,
    }
    #[async_trait]
    impl AgentEventSink for RecordSink {
        async fn emit(&self, event: AgentEvent) {
            self.events.lock().unwrap().push(event);
        }
    }
    let sink = RecordSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &DesignationProvider,
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
    .expect("designation completes the run");
    assert_eq!(
        output.answer, "The answer is 42.",
        "the prose answer is unchanged"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the designated SQL is carried on the output"
    );
    let streamed = events.lock().unwrap().clone();
    assert!(
        streamed.iter().any(|event| matches!(
            event,
            AgentEvent::AnswerDesignated { sql } if sql == "SELECT count(*) FROM t"
        )),
        "an AnswerDesignated event must reach the stream: {streamed:?}"
    );
    assert!(
        streamed
            .iter()
            .any(|event| matches!(event, AgentEvent::Complete)),
        "designation still ends with Complete: {streamed:?}"
    );
    // A normal turn (no designation) carries no SQL — the protocol is optional.
    let plain = run_agent(
        &MockProvider {
            responses: Mutex::new(vec![ChatResponse::new(ChatMessage::text(
                "assistant",
                "plain answer",
            ))]),
        },
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert!(
        plain.answer_sql.is_none(),
        "a turn without designation carries no SQL"
    );
}
