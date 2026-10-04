use super::*;
use saya_agent::REQUEST_CLARIFICATION_TOOL;

#[tokio::test]
async fn designation_recovery_shares_aggregate_headroom_across_every_reply() {
    let sql = "SELECT ".to_owned() + &"x".repeat(240);
    let calls = (0..8)
        .map(|index| {
            call(
                format!("designation-{index}"),
                "designate_answer",
                serde_json::json!({"sql":sql}),
            )
        })
        .collect();
    let (provider, requests) = ScriptedProvider::new(vec![
        turn("", calls),
        turn("the requested answer", Vec::new()),
    ]);
    let limits = AgentLimits {
        context_byte_budget: 16 * 1024,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &RecordingExecutor::default(),
        request("answer this", None),
        vec![definition("designate_answer")],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 64 + 4_000), 64),
        ),
    )
    .await
    .unwrap();

    assert_eq!(output.answer, "the requested answer");
    assert_eq!(output.answer_sql.as_deref(), Some(sql.as_str()));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let recovery = &requests[1];
    assert!(provider_projection_bytes(recovery) <= 4_000);
    assert!(
        recovery
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .map(|message| message.content.len())
            .sum::<usize>()
            <= 400
    );
    for index in 0..8 {
        let id = format!("designation-{index}");
        assert_eq!(
            recovery
                .messages
                .iter()
                .filter(|message| {
                    message.role == "tool" && message.tool_call_id.as_deref() == Some(id.as_str())
                })
                .count(),
            1,
            "each valid call receives exactly one paired result: {id}"
        );
    }
}

#[tokio::test]
async fn clarification_feed_keeps_all_paired_calls_in_the_reserved_request() {
    let mut calls = vec![call(
        "malformed-ask",
        REQUEST_CLARIFICATION_TOOL,
        serde_json::json!({"question":""}),
    )];
    calls.extend((0..11).map(|index| {
        call(
            format!("sibling-{index}"),
            "query",
            serde_json::json!({"sql":"SELECT 1"}),
        )
    }));
    let (provider, requests) =
        ScriptedProvider::new(vec![turn("", calls), turn("finished", Vec::new())]);
    let executor = RecordingExecutor::default();
    let limits = AgentLimits {
        context_byte_budget: 16 * 1024,
        ..AgentLimits::default()
    };
    let mut clarification = definition(REQUEST_CLARIFICATION_TOOL);
    clarification.effect.requires_approval = true;
    let output = run(
        &provider,
        &executor,
        request("clarify safely", None),
        vec![definition("query"), clarification],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 64 + 5_000), 64),
        ),
    )
    .await
    .unwrap();

    assert_eq!(output.answer, "finished");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let retry = &requests[1];
    assert!(provider_projection_bytes(retry) <= 5_000);
    assert!(
        retry
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .map(|message| message.content.len())
            .sum::<usize>()
            <= 900
    );
    for id in std::iter::once("malformed-ask".to_owned())
        .chain((0..11).map(|index| format!("sibling-{index}")))
    {
        assert_eq!(
            retry
                .messages
                .iter()
                .filter(|message| {
                    message.role == "tool" && message.tool_call_id.as_deref() == Some(id.as_str())
                })
                .count(),
            1,
            "each valid call receives exactly one paired result: {id}"
        );
    }
}

#[tokio::test]
async fn clarification_execution_receives_its_batch_share_and_refuses_siblings() {
    let mut calls = vec![call(
        "asked",
        REQUEST_CLARIFICATION_TOOL,
        serde_json::json!({"question":"Which source?","options":["A","B"]}),
    )];
    calls.extend((0..3).map(|index| {
        call(
            format!("paused-{index}"),
            REQUEST_CLARIFICATION_TOOL,
            serde_json::json!({"question":format!("sibling {index}")}),
        )
    }));
    let (provider, requests) = ScriptedProvider::new(vec![turn("", calls)]);
    let executor = RecordingExecutor::with_result(serde_json::json!({
        "rows": ["large clarification tool result".repeat(400)]
    }));
    let limits = AgentLimits {
        context_byte_budget: 16 * 1024,
        ..AgentLimits::default()
    };
    let output = run(
        &provider,
        &executor,
        request("need a question", None),
        vec![definition(REQUEST_CLARIFICATION_TOOL)],
        control(
            limits,
            &RecordingApproval::default(),
            &RecordingSink::default(),
            CancellationToken::new(),
            preflight(Some(1024 + 64 + 4_000), 64),
        ),
    )
    .await
    .unwrap();

    assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    let caps = executor.caps.lock().unwrap();
    assert_eq!(caps.len(), 1);
    assert!(
        caps[0] > 0 && caps[0] <= 200,
        "per-call cap was {}",
        caps[0]
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(output.events.iter().any(|event| matches!(
        event,
        AgentEvent::ClarificationNeeded { question, .. } if question == "Which source?"
    )));
}
