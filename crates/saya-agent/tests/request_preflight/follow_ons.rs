use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Continuation,
    Designation,
    Salvage,
}

#[tokio::test]
async fn continuation_designation_and_salvage_share_the_pre_dispatch_guard() {
    for route in [Route::Continuation, Route::Designation, Route::Salvage] {
        let usage = TokenUsage::new(17, 9);
        let (turns, definitions, mut limits, expected_window) = match route {
            Route::Continuation => (
                vec![vec![Err(ProviderError::output_truncated(
                    "partial answer".into(),
                    Vec::new(),
                ))]],
                Vec::new(),
                AgentLimits::default(),
                400,
            ),
            Route::Designation => (
                vec![
                    turn_with_usage(
                        "",
                        vec![call(
                            "designation-id",
                            "designate_answer",
                            serde_json::json!({"sql": "SELECT ".to_owned() + &"1".repeat(2_500)}),
                        )],
                        usage,
                    ),
                    turn("must not dispatch", Vec::new()),
                ],
                vec![definition("designate_answer")],
                AgentLimits::default(),
                4_000,
            ),
            Route::Salvage => {
                let limits = AgentLimits {
                    max_turns: Some(1),
                    ..AgentLimits::default()
                };
                (
                    vec![
                        turn_with_usage(
                            "prior answer",
                            vec![call(
                                "salvage-id",
                                "query",
                                serde_json::json!({"sql": "SELECT ".to_owned() + &"1".repeat(180)}),
                            )],
                            usage,
                        ),
                        turn("must not dispatch", Vec::new()),
                    ],
                    vec![definition("query")],
                    limits,
                    855,
                )
            }
        };
        limits.context_byte_budget = 16 * 1024;
        let (provider, requests) = ScriptedProvider::new(turns);
        let executor = RecordingExecutor::with_result(serde_json::json!({
            "columns":["value"],"rows":[[42]],"row_count":1,
            "truncated":false,"executed_sql":"SELECT 1"
        }));
        let sink = RecordingSink::default();
        let output = run(
            &provider,
            &executor,
            request(
                "same user question",
                matches!(route, Route::Salvage).then(|| "required system ".repeat(3)),
            ),
            definitions,
            control(
                limits,
                &RecordingApproval::default(),
                &sink,
                CancellationToken::new(),
                preflight(Some(1024 + 64 + expected_window), 64),
            ),
        )
        .await
        .unwrap();
        match route {
            Route::Continuation => {
                assert!(stopped(&output, BudgetDimension::RequestContextWindow));
                assert_eq!(requests.lock().unwrap().len(), 1);
                assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
            }
            Route::Designation => {
                assert_eq!(output.answer, "must not dispatch");
                let requests = requests.lock().unwrap();
                assert_eq!(requests.len(), 2);
                assert!(
                    requests
                        .iter()
                        .all(|request| provider_projection_bytes(request) <= 4_000)
                );
                assert_eq!(
                    sink.observations
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|observation| {
                            **observation == AnsweringObservation::RequestAdmitted
                        })
                        .count(),
                    2,
                    "the designation recovery receives the same preflight before admission"
                );
                assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
                assert_eq!(
                    output.answer_sql.as_deref(),
                    Some(("SELECT ".to_owned() + &"1".repeat(2_500)).as_str())
                );
                assert_eq!(output.usage, usage);
            }
            Route::Salvage => {
                assert!(
                    stopped(&output, BudgetDimension::RequestContextWindow),
                    "events {:?}; requests {}",
                    output.events,
                    requests.lock().unwrap().len()
                );
                assert_eq!(requests.lock().unwrap().len(), 1);
                assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
                assert_eq!(output.tool_metadata.len(), 1);
                assert_eq!(output.tool_metadata[0].status, "completed");
                let expected_sql = "SELECT ".to_owned() + &"1".repeat(180);
                assert_eq!(output.answer_sql.as_deref(), Some(expected_sql.as_str()));
                assert_eq!(output.usage, usage);
            }
        }
    }
}
