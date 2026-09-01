use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentEvent, AgentEventSink, AgentLimits, AgentRequest, AllowReadOnlyApproval,
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    ProviderEvent, ProviderStream, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
    run_agent, run_agent_with_sink,
};
use std::sync::{Arc, Mutex};

struct MockProvider {
    responses: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl ChatProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }
    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        assert_eq!(request.model, "mock-model");
        self.responses.lock().unwrap().remove(0).pipe(Ok)
    }
}

struct MockTools {
    calls: Arc<Mutex<Vec<String>>>,
}

struct HistoryProvider {
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for HistoryProvider {
    fn name(&self) -> &str {
        "history-mock"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        Ok(ChatResponse {
            message: ChatMessage::text("assistant", "second answer"),
        })
    }
}

struct DenyApproval;

#[async_trait]
impl ApprovalDecider for DenyApproval {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        false
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[async_trait]
impl ToolExecutor for MockTools {
    async fn execute(
        &self,
        name: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.into());
        Ok(serde_json::json!({"rows": 1}))
    }
}

fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "bounded_sql_query".into(),
            description: "read-only query".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: saya_agent::LocalStateEffect::None,
            },
        },
        ToolDefinition {
            name: "bounded_sql_query_all".into(),
            description: "fan-out read-only query".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: saya_agent::LocalStateEffect::None,
            },
        },
        ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema discovery".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: saya_agent::LocalStateEffect::None,
            },
        },
    ]
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

#[tokio::test]
async fn prior_user_and_assistant_turn_reaches_provider_in_order() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = HistoryProvider {
        requests: requests.clone(),
    };
    let mut request = request();
    request.history = vec![
        ChatMessage::text("user", "first prompt"),
        ChatMessage::text("assistant", "first answer"),
    ];
    run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request,
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    let captured = requests.lock().unwrap();
    assert_eq!(captured[0].messages[1].content, "first prompt");
    assert_eq!(captured[0].messages[2].content, "first answer");
    assert_eq!(captured[0].messages[3].content, "show data");
}

#[tokio::test]
async fn tool_call_round_trip_is_deterministic_and_emits_safe_events() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select 1"}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "There is one result."),
            },
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "There is one result.");
    assert_eq!(&*calls.lock().unwrap(), &["bounded_sql_query"]);
    assert!(output.used_bounded_sql_query);
    assert_eq!(output.tool_metadata[0].name, "bounded_sql_query");
    assert_eq!(output.tool_metadata[0].status, "completed");
    assert!(output.events.iter().any(|event| matches!(event, saya_agent::AgentEvent::ToolCompleted { summary, .. } if summary.contains("read-only"))));
}

#[tokio::test]
async fn tool_call_limits_stop_run_before_unbounded_execution() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "call-1".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            },
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: 1,
            max_tool_calls: 0,
            permit_candidate_writes: false,
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::Limit("tool calls")));
}

#[tokio::test]
async fn unknown_tool_call_feeds_an_error_result_and_the_turn_recovers() {
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "shell".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "recovered"),
            },
        ]),
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
    .unwrap();
    // The hallucinated tool never executed, the model saw an error result,
    // and it corrected itself instead of the run aborting.
    assert_eq!(output.answer, "recovered");
    assert!(tools.calls.lock().unwrap().is_empty());
    assert!(
        output
            .tool_metadata
            .iter()
            .any(|item| item.name == "shell" && item.status == "failed")
    );
}

#[tokio::test]
async fn non_object_tool_arguments_are_recovered_not_fatal() {
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!("not-an-object"),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "recovered"),
            },
        ]),
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
    .unwrap();
    assert_eq!(output.answer, "recovered");
    assert!(tools.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn missing_tool_call_id_remains_fail_closed() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: String::new(),
                    name: "shell".into(),
                    arguments: serde_json::json!({}),
                }],
                tool_call_id: None,
            },
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    // Without a call id there is no way to anchor a well-formed tool result,
    // so the conversation could not continue validly.
    assert!(matches!(error, AgentError::InvalidToolCall));
}

#[tokio::test]
async fn empty_provider_response_is_invalid() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse {
            message: ChatMessage::text("assistant", ""),
        }]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(saya_agent::ProviderError::InvalidResponse)
    ));
}

#[tokio::test]
async fn injected_denial_does_not_execute_query_or_persist_rows() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql":"select secret"}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "The query was denied."),
            },
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &DenyApproval,
    )
    .await
    .unwrap();
    assert!(calls.lock().unwrap().is_empty());
    assert!(!output.used_bounded_sql_query);
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolDenied { .. }))
    );
    assert!(!output.answer.contains("secret"));
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

struct StreamingToolProvider;
#[async_trait]
impl ChatProvider for StreamingToolProvider {
    fn name(&self) -> &str {
        "streaming-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, saya_agent::ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, saya_agent::ProviderError> {
        let call = ToolCall {
            id: "call-1".into(),
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql":"select 1"}),
        };
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(ProviderEvent::ToolCalls(vec![call])),
            Ok(ProviderEvent::Done),
        ])))
    }
}

struct CancellingSink {
    token: CancellationToken,
    events: Arc<Mutex<Vec<AgentEvent>>>,
}
#[async_trait]
impl AgentEventSink for CancellingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event.clone());
        if matches!(event, AgentEvent::ToolRequested { .. }) {
            self.token.cancel();
        }
    }
}

#[tokio::test]
async fn cancellation_blocks_tool_execution_and_terminal_events() {
    let token = CancellationToken::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let error = run_agent_with_sink(
        &StreamingToolProvider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &CancellingSink {
            token: token.clone(),
            events: events.clone(),
        },
        token,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::Cancelled));
    assert!(calls.lock().unwrap().is_empty());
    assert!(!events.lock().unwrap().iter().any(|event| matches!(
        event,
        AgentEvent::ToolCompleted { .. } | AgentEvent::Complete
    )));
}

#[tokio::test]
async fn bounded_sql_query_all_sets_flag_and_schema_discovery_does_not() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "bounded_sql_query_all".into(),
                        arguments: serde_json::json!({"sql":"select 1"}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "Results across databases."),
            },
        ]),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "Results across databases.");
    assert_eq!(&*calls.lock().unwrap(), &["bounded_sql_query_all"]);
    assert!(output.used_bounded_sql_query);

    let calls_schema = Arc::new(Mutex::new(Vec::new()));
    let provider_schema = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-2".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "Discovered schema."),
            },
        ]),
    };
    let output_schema = run_agent(
        &provider_schema,
        &MockTools {
            calls: calls_schema.clone(),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output_schema.answer, "Discovered schema.");
    assert_eq!(&*calls_schema.lock().unwrap(), &["schema_discovery"]);
    assert!(!output_schema.used_bounded_sql_query);
}

/// Proves approval-free tool calls in one assistant message overlap: each
/// executor entry waits at a 2-party barrier, so sequential execution would
/// deadlock (surfacing as a timeout) instead of passing.
#[tokio::test]
async fn approval_free_tool_calls_run_concurrently_and_results_stay_ordered() {
    use std::time::Duration;
    use tokio::sync::Barrier;

    struct BarrierTools {
        barrier: Arc<Barrier>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for BarrierTools {
        async fn execute(
            &self,
            name: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            self.barrier.wait().await;
            self.seen.lock().unwrap().push(name.into());
            Ok(serde_json::json!({"ok": true}))
        }
    }

    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![
                        ToolCall {
                            id: "call-a".into(),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({"which": 1}),
                        },
                        ToolCall {
                            id: "call-b".into(),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({"which": 2}),
                        },
                    ],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "parallel done"),
            },
        ]),
    };
    let tools = BarrierTools {
        barrier: Arc::new(Barrier::new(2)),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let run = tokio::time::timeout(
        Duration::from_secs(5),
        run_agent(
            &provider,
            &tools,
            request(),
            definitions(),
            AgentLimits::default(),
            &AllowReadOnlyApproval,
        ),
    )
    .await
    .expect("calls must overlap; sequential execution deadlocks at the barrier");
    let output = run.unwrap();
    assert_eq!(output.answer, "parallel done");
    assert_eq!(
        &*tools.seen.lock().unwrap(),
        &["schema_discovery", "schema_discovery"]
    );
}

/// Proves the fan-out in `execute_batch` is bounded: every executor entry
/// increments an in-flight counter, parks on a zero-permit semaphore so peers
/// can pile up, and records the high-water mark of simultaneous execution. The
/// test waits for in-flight to stabilise (the cap's worth are parked), reads
/// the high-water, and releases. Unbounded simultaneity drives the high-water
/// to N; a cap of C holds it at C.
#[tokio::test]
async fn execute_batch_caps_simultaneous_concurrency() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Semaphore;

    struct ProbeTools {
        in_flight: Arc<AtomicUsize>,
        high_water: Arc<AtomicUsize>,
        gate: Arc<Semaphore>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for ProbeTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            let cur = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            let mut seen = self.high_water.load(Ordering::SeqCst);
            while cur > seen {
                match self.high_water.compare_exchange(
                    seen,
                    cur,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => seen = actual,
                }
            }
            // Block on a zero-permit gate, holding the in-flight slot open so
            // the next call can only start once a slot frees. The test adds the
            // permits that release every parked call.
            let _permit = self.gate.acquire().await.expect("gate not closed");
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok(serde_json::json!({"ok": true}))
        }
    }

    const N: usize = 12;
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: (0..N)
                        .map(|index| ToolCall {
                            id: format!("call-{index}"),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({"which": index}),
                        })
                        .collect(),
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "done"),
            },
        ]),
    };
    let in_flight = Arc::new(AtomicUsize::new(0));
    let high_water = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Semaphore::new(0));
    let tools = ProbeTools {
        in_flight: in_flight.clone(),
        high_water: high_water.clone(),
        gate: gate.clone(),
    };
    let run = run_agent(
        &provider,
        &tools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: 4,
            max_tool_calls: 64,
            permit_candidate_writes: false,
            context_byte_budget: 1024 * 1024,
        },
        &AllowReadOnlyApproval,
    );
    // Drive the run while waiting for in-flight to stabilise — i.e. the cap's
    // worth are parked and the rest are queued behind the cap. Polling both
    // futures on this task avoids borrowing across a `tokio::spawn` boundary.
    let mut run = std::pin::pin!(run);
    let stabilised = tokio::select! {
        output = &mut run => { let _ = output; false },
        _ = async {
            let mut last = 0;
            let mut stable = 0;
            loop {
                let cur = in_flight.load(Ordering::SeqCst);
                if cur == last && cur > 0 {
                    stable += 1;
                    if stable >= 3 {
                        return;
                    }
                } else {
                    stable = 0;
                }
                last = cur;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        } => true,
    };
    assert!(
        stabilised,
        "in-flight must stabilise under the cap, not finish"
    );
    let observed = high_water.load(Ordering::SeqCst);
    // Release every parked call so the run can drain and finish. Permits
    // accumulate, so one add covers all N calls regardless of how the cap
    // batches them.
    gate.add_permits(N);
    let output = tokio::time::timeout(Duration::from_secs(10), run)
        .await
        .expect("batch must not deadlock under the cap")
        .unwrap();
    assert_eq!(output.answer, "done");
    // Before the fix the high-water mark equalled N (all twelve ran at once);
    // the cap must hold the observed simultaneity strictly below N.
    assert!(
        observed < N,
        "concurrency must be capped below {N}; observed {observed} simultaneous"
    );
    assert!(
        observed >= 1,
        "at least one call must run; observed {observed}"
    );
}

/// Providers disclose cumulative counts per response; the run total must sum
/// them across turns and land in AgentOutput.
#[tokio::test]
async fn token_usage_sums_across_turns_into_the_output() {
    use saya_agent::{ProviderEvent, ProviderStream, TokenUsage};

    struct UsageProvider {
        turns: std::sync::Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl ChatProvider for UsageProvider {
        fn name(&self) -> &str {
            "usage"
        }
        async fn complete(
            &self,
            _: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            panic!("loop must drive providers through stream()");
        }
        async fn stream(
            &self,
            _: ChatRequest,
            _: CancellationToken,
        ) -> Result<ProviderStream, saya_agent::ProviderError> {
            let mut turns = self.turns.lock().unwrap();
            *turns += 1;
            let turn = *turns;
            drop(turns);
            let events = if turn == 1 {
                vec![
                    Ok(ProviderEvent::ToolCalls(vec![ToolCall {
                        id: "call-1".into(),
                        name: "schema_discovery".into(),
                        arguments: serde_json::json!({}),
                    }])),
                    Ok(ProviderEvent::Usage(TokenUsage {
                        input_tokens: 3,
                        output_tokens: 7,
                        ..Default::default()
                    })),
                    Ok(ProviderEvent::Done),
                ]
            } else {
                vec![
                    Ok(ProviderEvent::TextDelta("final answer".into())),
                    Ok(ProviderEvent::Usage(TokenUsage {
                        input_tokens: 5,
                        output_tokens: 9,
                        ..Default::default()
                    })),
                    Ok(ProviderEvent::Done),
                ]
            };
            Ok(Box::pin(futures_util::stream::iter(events)))
        }
    }

    let output = run_agent(
        &UsageProvider {
            turns: std::sync::Mutex::new(0),
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
    assert_eq!(output.answer, "final answer");
    assert_eq!(
        output.usage,
        TokenUsage {
            input_tokens: 8,
            output_tokens: 16,
            ..Default::default()
        }
    );
}

/// The intra-loop context bound still binds, but it no longer aborts the run
/// (S4 Problem A): runaway context is trimmed to fit rather than surfacing an
/// opaque `Limit("context bytes")` after the query already ran. Each provider
/// turn is issued a conversation assembled under the byte budget; the run
/// completes instead of dying on the first oversized turn.
#[tokio::test]
async fn runaway_context_is_trimmed_not_aborted_and_the_bound_still_binds() {
    struct CapturingProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for CapturingProvider {
        fn name(&self) -> &str {
            "capture"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            let turn = {
                let mut t = self.turn.lock().unwrap();
                let was = *t;
                *t += 1;
                was
            };
            if turn < 4 {
                return Ok(ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: format!("call-{turn}"),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({"padding": "x".repeat(2_048)}),
                        }],
                        tool_call_id: None,
                    },
                });
            }
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", "done"),
            })
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CapturingProvider {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: 8,
            max_tool_calls: 64,
            permit_candidate_writes: false,
            context_byte_budget: 4_096,
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("runaway context is trimmed, not aborted");
    assert_eq!(output.answer, "done");
    // The bound still binds: every turn's assembled conversation fits the byte
    // budget. This is the invariant the old aborting test guarded — the
    // mechanism changed from fail-closed to trim, the bound did not go away.
    let budget = 4_096usize;
    for request in requests.lock().unwrap().iter() {
        let total = request
            .messages
            .iter()
            .map(|message| {
                message.content.len() + message.role.len() + message_size_overhead(message)
            })
            .sum::<usize>();
        assert!(
            total <= budget,
            "each turn must stay within the {budget}-byte budget; saw {total}"
        );
    }
}

/// Approximates the loop's `message_size` for assertion purposes (content plus
/// role plus tool-call argument bytes), matching what the bound measures.
fn message_size_overhead(message: &ChatMessage) -> usize {
    message
        .tool_calls
        .iter()
        .map(|call| {
            call.id.len()
                + call.name.len()
                + serde_json::to_string(&call.arguments).map_or(0, |text| text.len())
        })
        .sum::<usize>()
}

/// A single oversized tool result must not be able to abort the whole run by
/// itself (S4 invariant 1). One query returning more than the conversation
/// byte budget must let the run continue in some useful form rather than
/// surfacing an opaque `Limit("context bytes")` after the query already ran.
/// The model must also be told it received a cut result, not silently fed a
/// partial one as if complete (S4: a correctness issue, not just UX).
#[tokio::test]
async fn single_oversized_tool_result_does_not_abort_the_run() {
    struct BigTools;
    #[async_trait::async_trait]
    impl ToolExecutor for BigTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            // One result larger than the whole conversation budget.
            Ok(serde_json::json!({ "rows": vec!["x".repeat(8_192)] }))
        }
    }
    struct CapturingProvider {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
        turn: Mutex<usize>,
    }
    #[async_trait]
    impl ChatProvider for CapturingProvider {
        fn name(&self) -> &str {
            "capture-big"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            self.requests.lock().unwrap().push(request);
            let turn = {
                let mut t = self.turn.lock().unwrap();
                let was = *t;
                *t += 1;
                was
            };
            if turn == 0 {
                return Ok(ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({}),
                        }],
                        tool_call_id: None,
                    },
                });
            }
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", "summarised the result"),
            })
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = CapturingProvider {
        requests: requests.clone(),
        turn: Mutex::new(0),
    };
    let output = run_agent(
        &provider,
        &BigTools,
        request(),
        definitions(),
        AgentLimits {
            max_turns: 4,
            max_tool_calls: 8,
            permit_candidate_writes: false,
            context_byte_budget: 4_096,
        },
        &AllowReadOnlyApproval,
    )
    .await
    // Before the fix this returned `Err(AgentError::Limit("context bytes"))`.
    .expect("one oversized result must not abort the run");
    assert_eq!(output.answer, "summarised the result");
    // The model must not silently believe it saw the complete result.
    assert!(
        output.events.iter().any(
            |event| matches!(event, AgentEvent::ToolCompleted { summary, .. } if summary
                .contains("truncated"))
        ),
        "the model must be told the result was truncated; events: {:?}",
        output.events
    );
    // And the tool message the model actually receives must carry the visible
    // truncation marker, proving it is not misled about what it got.
    let second = requests.lock().unwrap()[1].clone();
    let tool_message = second
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("the truncated result must reach the model as a tool message");
    assert!(
        tool_message.content.contains("truncated"),
        "the tool message content must mark the cut, got: {}",
        tool_message.content
    );
}

/// A failing tool's error text must reach the model as the tool result so it
/// can adjust (e.g. a safety-rejection reason), not be flattened to a generic
/// "database tool failed" blob.
#[tokio::test]
async fn tool_failure_details_reach_the_model() {
    struct CapturingProvider2 {
        requests: Arc<Mutex<Vec<ChatRequest>>>,
    }

    struct FailingTools;

    #[async_trait::async_trait]
    impl ToolExecutor for FailingTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Err(ToolError::QueryFailedDetail(
                "query rejected: DELETE modifies data".into(),
            ))
        }
    }

    #[async_trait::async_trait]
    impl ChatProvider for CapturingProvider2 {
        fn name(&self) -> &str {
            "capture"
        }
        async fn complete(
            &self,
            request: ChatRequest,
        ) -> Result<ChatResponse, saya_agent::ProviderError> {
            let mut turns = self.requests.lock().unwrap();
            let turn = turns.len();
            turns.push(request);
            if turn == 0 {
                return Ok(ChatResponse {
                    message: ChatMessage {
                        role: "assistant".into(),
                        content: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call-1".into(),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({}),
                        }],
                        tool_call_id: None,
                    },
                });
            }
            Ok(ChatResponse {
                message: ChatMessage::text("assistant", "adjusted"),
            })
        }
    }

    let provider = CapturingProvider2 {
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let output = run_agent(
        &provider,
        &FailingTools,
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "adjusted");
    let second = provider.requests.lock().unwrap()[1].clone();
    let tool_message = second
        .messages
        .iter()
        .find(|message| message.role == "tool")
        .expect("the failure must be fed back as a tool result");
    assert!(
        tool_message.content.contains("DELETE modifies data"),
        "model must see the underlying reason, got: {}",
        tool_message.content
    );
}

/// A tool that declares an external side effect but does *not* require
/// approval is a misconfiguration the policy must gate — never auto-run. The
/// two execution paths (sequential, when the call arrives alone, and the
/// concurrent batch, when it arrives among others) must agree: the gated tool
/// is denied in both, never silently executed. This is the divergence guard
/// for S8: if the batch predicate and the sequential path stop consulting the
/// same policy, one of these assertions fails.
fn external_side_effect_without_approval_tool() -> ToolDefinition {
    ToolDefinition {
        name: "open_browser".into(),
        description: "opens something outside the agent".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::None,
        },
    }
}

/// Asserts `open_browser` neither executed nor completed, and was denied with
/// a non-empty reason — the observable shape of "the policy gated this call".
fn assert_open_browser_gated(calls: &[String], events: &[AgentEvent]) {
    assert!(
        !calls.iter().any(|name| name == "open_browser"),
        "the gated tool must not execute; got calls {calls:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCompleted { name, .. } if name == "open_browser"
        )),
        "the gated tool must not complete; got events {events:?}"
    );
    let denied = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "open_browser" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("the gated tool must surface a ToolDenied event with a reason");
    assert!(
        !denied.is_empty(),
        "the denial must carry a clear reason, not be a silent skip"
    );
}

/// The gated tool arriving alone takes the sequential path: it must be
/// denied, not executed. Against the pre-S8 code the sequential path ignored
/// `external_side_effect`, so this assertion fails there (the tool ran).
#[tokio::test]
async fn external_side_effect_tool_is_gated_when_it_arrives_alone() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        name: "open_browser".into(),
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "done"),
            },
        ]),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        vec![external_side_effect_without_approval_tool()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert_open_browser_gated(&calls.lock().unwrap().clone(), &events.lock().unwrap());
    assert_eq!(output.tool_metadata[0].name, "open_browser");
    assert_eq!(output.tool_metadata[0].status, "denied");
}

/// The gated tool arriving in a batch with an auto-runnable call must still be
/// gated: the batch is not run concurrently for it, it falls through to the
/// sequential path and is denied, while the auto-runnable sibling executes.
/// Against the pre-S8 code the batch predicate already excluded the gated
/// tool (it tests `external_side_effect`), so the batch fell through — but the
/// sequential path then *ran* it, since it ignored `external_side_effect`. So
/// the "must not execute" assertion fails there.
#[tokio::test]
async fn external_side_effect_tool_is_gated_when_it_arrives_in_a_batch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let provider = MockProvider {
        responses: Mutex::new(vec![
            ChatResponse {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: String::new(),
                    tool_calls: vec![
                        ToolCall {
                            id: "call-a".into(),
                            name: "open_browser".into(),
                            arguments: serde_json::json!({}),
                        },
                        ToolCall {
                            id: "call-b".into(),
                            name: "schema_discovery".into(),
                            arguments: serde_json::json!({}),
                        },
                    ],
                    tool_call_id: None,
                },
            },
            ChatResponse {
                message: ChatMessage::text("assistant", "done"),
            },
        ]),
    };
    let definitions = {
        let mut defs = definitions();
        defs.push(external_side_effect_without_approval_tool());
        defs
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let _ = run_agent_with_sink(
        &provider,
        &MockTools {
            calls: calls.clone(),
        },
        request(),
        definitions,
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("run completes");
    let calls = calls.lock().unwrap().clone();
    let events = events.lock().unwrap().clone();
    assert_open_browser_gated(&calls, &events);
    // The auto-runnable sibling is unaffected: it executes normally.
    assert!(
        calls.iter().any(|name| name == "schema_discovery"),
        "the non-gated sibling must still execute; got calls {calls:?}"
    );
}
