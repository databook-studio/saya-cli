const TOOL_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-sql\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"connection\\\":\\\"analytics\\\",\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
const ANSWER_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"The query returned one.\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";
const EXTRACTION_RESPONSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"proposals\\\": []}\"},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\ndata: [DONE]\n\n";

#[tokio::test]
async fn pre_stopped_question_never_opens_candidate_setup_connection() {
    for candidates in [1, 2] {
        let root = temp_root();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut runtime = runtime_for_postgres_setup(&root, port).await;
        runtime.resolved.query_timeout_seconds = 1;
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let scope =
            crate::agent::question_scope::QuestionScope::new(&runtime, cancellation.clone());
        let sink = RecordingSink {
            events: Arc::new(Mutex::new(Vec::new())),
            knowledge_log: Arc::new(Mutex::new(Vec::new())),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(4),
            crate::agent::candidates::run_with_candidates(
                &runtime,
                "do not start a stopped question",
                ApprovalPolicy::ReadOnly,
                false,
                false,
                PromptOverrides::default(),
                Vec::new(),
                &sink,
                scope,
                None,
                None,
                None,
                None,
                AgentMode::Build,
                candidates,
            ),
        )
        .await
        .expect("stopped candidate entry settles without source work");
        assert!(result.is_err());
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a stopped question must not open a new setup connection ({candidates} candidates)"
        );
        assert!(sink.events.lock().unwrap().is_empty());
        assert_eq!(
            cancellation.reason(),
            Some(saya_agent::CancellationReason::User)
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
