/// A run that exhausts its turn budget with a successful query and no
/// nomination ends with that query nominated. The model never called
/// `designate_answer` (the budget ran out first), so the runner surfaces the
/// last statement that completed successfully. Against the code before the
/// fix `answer_sql` was `None` even though a query succeeded.
#[tokio::test]
async fn budget_exhaustion_with_a_successful_query_nominates_it() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: false,
        error: String::new(),
    };
    let (output, _calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT count(*) FROM t"),
            ChatMessage::text("assistant", "best effort from gathered results"),
        ],
        executor,
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
    )
    .await;
    assert!(
        output.truncated,
        "a budget-exhausted run must be marked truncated"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the last successful statement is nominated when the model never did"
    );
}

/// A run whose only statements failed nominates nothing — a wrong nomination
/// is worse than an absent one. Only successful statements are nomination
/// candidates, so an all-failure run leaves `answer_sql` absent.
#[tokio::test]
async fn budget_exhaustion_with_only_failed_statements_nominates_nothing() {
    let executor = ScriptedExecutor {
        calls: Arc::new(Mutex::new(Vec::new())),
        fail: true,
        error: "bad".into(),
    };
    let (output, _calls, _requests) = run(
        vec![
            tool_turn("c1", "SELECT bad"),
            ChatMessage::text("assistant", "best effort"),
        ],
        executor,
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
    )
    .await;
    assert!(
        output.truncated,
        "a budget-exhausted run must be marked truncated"
    );
    assert!(
        output.answer_sql.is_none(),
        "an all-failure run must nominate nothing, got {:?}",
        output.answer_sql
    );
}
