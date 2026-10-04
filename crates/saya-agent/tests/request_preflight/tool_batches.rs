use super::*;

#[tokio::test]
async fn whole_batch_refusal_skeleton_blocks_all_effects_and_keeps_usage() {
    let calls = (0..2)
        .map(|index| {
            call(
                format!("call-{index}-{}", "i".repeat(180)),
                "query",
                serde_json::json!({"sql": "S".repeat(220)}),
            )
        })
        .collect();
    let (provider, requests) = ScriptedProvider::new(vec![turn_with_usage(
        "partial evidence",
        calls,
        TokenUsage::new(21, 8),
    )]);
    let executor = RecordingExecutor::default();
    let approval = RecordingApproval::default();
    let sink = RecordingSink::default();
    let limits = AgentLimits {
        context_byte_budget: 1_100,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &executor,
        request("ask", None),
        vec![definition("query")],
        control(
            limits,
            &approval,
            &sink,
            CancellationToken::new(),
            preflight(None, 32),
        ),
    )
    .await
    .unwrap();
    assert!(stopped(&output, BudgetDimension::RequestContextBytes));
    assert_eq!(output.answer, "partial evidence");
    assert_eq!(output.usage, TokenUsage::new(21, 8));
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    assert_eq!(approval.0.load(Ordering::SeqCst), 0);
    assert!(
        sink.observations
            .lock()
            .unwrap()
            .contains(&AnsweringObservation::ToolBatchObserved { requested: 2 })
    );
}

#[tokio::test]
async fn fitting_batch_executes_with_result_cap_from_remaining_request_headroom() {
    let (provider, requests) = ScriptedProvider::new(vec![
        turn(
            "working",
            vec![call(
                "fit-1",
                "query",
                serde_json::json!({"sql":"SELECT 1"}),
            )],
        ),
        turn("finished", Vec::new()),
    ]);
    let executor = RecordingExecutor::with_result(serde_json::json!({
        "columns": ["value"],
        "rows": [["large result".repeat(400)]],
        "row_count": 1,
        "truncated": false,
        "executed_sql": "SELECT 1"
    }));
    let limits = AgentLimits {
        context_byte_budget: 32 * 1024,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &executor,
        request("ask", None),
        vec![definition("query")],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 128 + 2_000), 128),
        ),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "finished");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let caps = executor.caps.lock().unwrap();
    assert_eq!(caps.len(), 1);
    assert!(caps[0] > 0 && caps[0] < limits.context_byte_budget);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| { provider_projection_bytes(request) <= 2_000 })
    );
}
