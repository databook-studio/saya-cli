struct ToolThenAnswer {
    calls: Arc<Mutex<usize>>,
}

#[async_trait]
impl ChatProvider for ToolThenAnswer {
    fn name(&self) -> &str {
        "tool-then-answer"
    }

    async fn complete(
        &self,
        _request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            return Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "first".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": "SELECT 1"}),
                }],
                tool_call_id: None,
            }));
        }
        Ok(ChatResponse::new(ChatMessage::text(
            "assistant",
            "second request",
        )))
    }
}

/// The configured C3 request wallet belongs at the CLI runtime composition
/// boundary: after a test connector tool call, a provider's second answering
/// request must be refused rather than inheriting the legacy unbounded loop.
#[tokio::test]
async fn runtime_uses_configured_request_wallet_after_real_sql_tool() {
    let calls = Arc::new(Mutex::new(0));
    let inputs = TurnInputs {
        ai: test_ai(16 * 1024),
        provider: Box::new(ToolThenAnswer {
            calls: Arc::clone(&calls),
        }),
        registry: registry_for("analytics"),
        failures: Vec::new(),
    };
    let mut runtime = test_runtime(16 * 1024);
    runtime
        .resolved
        .investigation_budgets
        .logical_answering_requests =
        BudgetLimit::Finite(NonZeroU64::new(1).expect("one is a valid finite request cap"));
    let result = run_prompt_with_inputs(
        &runtime,
        inputs,
        "count rows",
        ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &NoopEventSink,
        CancellationToken::new(),
        None,
        None,
        None,
        None,
        AgentMode::Build,
        None,
        None,
    )
    .await;
    let output = result.expect("a budget stop preserves the loop's partial output");
    assert!(
        output.events.iter().any(|event| matches!(
            event,
            AgentEvent::AnsweringBudgetStopped {
                dimension: saya_agent::BudgetDimension::LogicalRequests
            }
        )),
        "the typed partial output must name the configured request budget"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        1,
        "the runtime must not send the provider a second request after real SQL"
    );
}

struct TwoSqlCallsThenAnswer {
    calls: Arc<Mutex<usize>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for TwoSqlCallsThenAnswer {
    fn name(&self) -> &str {
        "two-sql-calls-then-answer"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            return Ok(ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![
                    ToolCall {
                        id: "first".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql": "SELECT id FROM orders"}),
                    },
                    ToolCall {
                        id: "second".into(),
                        name: "bounded_sql_query".into(),
                        arguments: serde_json::json!({"sql": "SELECT count(*) FROM orders"}),
                    },
                ],
                tool_call_id: None,
            }));
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

fn successful_sql_results(requests: &[ChatRequest], expected: usize) {
    let follow_up = requests
        .get(1)
        .expect("successful SQL calls produce one paired provider request");
    let results: Vec<QueryResult> = follow_up
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .map(|message| serde_json::from_str(&message.content).expect("tool result is JSON"))
        .collect();
    assert_eq!(
        results.len(),
        expected,
        "one successful result per SQL call"
    );
    assert!(results.iter().all(|result| {
        result.row_count == 1
            && !result.truncated
            && result.rows == vec![serde_json::json!([1])]
            && result.executed_sql.contains("orders")
    }));
}

#[tokio::test]
async fn runtime_composes_requested_tool_limit_with_production_sqlite_tools() {
    for (requested_tool_calls, executed) in [(1, 0), (3, 2)] {
        let root = std::env::temp_dir().join(format!(
            "saya-runtime-tool-budget-{}-{requested_tool_calls}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create SQLite budget root");
        let calls = Arc::new(Mutex::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inputs = TurnInputs {
            ai: test_ai(16 * 1024),
            provider: Box::new(TwoSqlCallsThenAnswer {
                calls: Arc::clone(&calls),
                requests: Arc::clone(&requests),
            }),
            registry: sqlite_registry(&root).await,
            failures: Vec::new(),
        };
        let mut runtime = test_runtime(16 * 1024);
        runtime.resolved.investigation_budgets.requested_tool_calls =
            BudgetLimit::Finite(NonZeroU64::new(requested_tool_calls).unwrap());
        let output = run_prompt_with_inputs(
            &runtime,
            inputs,
            "count rows",
            ApprovalPolicy::ReadOnly,
            false,
            false,
            Vec::new(),
            &NoopEventSink,
            CancellationToken::new(),
            None,
            None,
            None,
            None,
            AgentMode::Build,
            None,
            None,
        )
        .await
        .expect("the runtime returns its partial or completed output");
        let completed = output
            .events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query"))
            .count();
        assert_eq!(
            completed, executed,
            "configured tool budget controls real SQLite execution"
        );
        assert_eq!(*calls.lock().unwrap(), if executed == 0 { 1 } else { 2 });
        if executed == 0 {
            assert!(output.events.iter().any(|event| matches!(
                event,
                AgentEvent::AnsweringBudgetStopped {
                    dimension: saya_agent::BudgetDimension::RequestedToolCalls
                }
            )));
            assert_eq!(
                requests.lock().unwrap().len(),
                1,
                "no paired SQL result is sent"
            );
        } else {
            successful_sql_results(&requests.lock().unwrap(), executed);
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
