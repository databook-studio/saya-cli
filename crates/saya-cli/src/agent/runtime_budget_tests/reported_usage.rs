struct ReportedUsageThenSql {
    calls: Arc<Mutex<usize>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl ChatProvider for ReportedUsageThenSql {
    fn name(&self) -> &str {
        "reported-usage-then-sql"
    }

    async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, saya_agent::ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            let mut response = ChatResponse::new(ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "sql".into(),
                    name: "bounded_sql_query".into(),
                    arguments: serde_json::json!({"sql": "SELECT id FROM orders"}),
                }],
                tool_call_id: None,
            });
            response.usage = Some(saya_agent::TokenUsage::new(1, 0));
            return Ok(response);
        }
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

#[tokio::test]
async fn runtime_composes_reported_token_limit_with_production_sqlite_tools() {
    for (known_reported_tokens, executed) in [(1, 0), (2, 1)] {
        let root = std::env::temp_dir().join(format!(
            "saya-runtime-token-budget-{}-{known_reported_tokens}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create SQLite token budget root");
        let calls = Arc::new(Mutex::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inputs = TurnInputs {
            ai: test_ai(16 * 1024),
            provider: Box::new(ReportedUsageThenSql {
                calls: Arc::clone(&calls),
                requests: Arc::clone(&requests),
            }),
            registry: sqlite_registry(&root).await,
            failures: Vec::new(),
        };
        let mut runtime = test_runtime(16 * 1024);
        runtime.resolved.investigation_budgets.known_reported_tokens =
            BudgetLimit::Finite(NonZeroU64::new(known_reported_tokens).unwrap());
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
            "reported usage gates real SQLite execution"
        );
        assert_eq!(*calls.lock().unwrap(), if executed == 0 { 1 } else { 2 });
        if executed == 0 {
            assert!(output.events.iter().any(|event| matches!(
                event,
                AgentEvent::AnsweringBudgetStopped {
                    dimension: saya_agent::BudgetDimension::ReportedTokens
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
