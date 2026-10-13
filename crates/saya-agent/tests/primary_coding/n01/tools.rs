use std::sync::atomic::Ordering;

use saya_agent::{
    AgentError, CancellationReason, CancellationToken, EffectOutcome, ToolConcurrency,
    run_agent_with_control,
};

use super::{
    common::{ApproveAll, control, effect, request, tool, tool_call},
    executor::RecordingExecutor,
    owner::{OwnerOptions, RecordingOwner, model, tool as tool_settlement, trace},
    provider::{ScriptedProvider, answer, tool_calls},
};

#[tokio::test]
async fn denied_serial_tool_after_approval_never_reaches_executor() {
    let trace = trace();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            deny_external_tool: true,
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![tool_call("approved-write", "call-1")])],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definition = tool(
        "approved-write",
        effect(false, true, true),
        ToolConcurrency::Serial,
    );

    let result =
        run_agent_with_control(&provider, &executor, request(), vec![definition], control).await;

    assert!(matches!(result, Err(AgentError::EffectAdmissionDenied)));
    assert_eq!(approval.calls.load(Ordering::SeqCst), 1);
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *trace.lock().unwrap(),
        ["admit:model", "provider", "settle:1", "admit:tools:1"]
    );
    assert_eq!(
        *owner.settlements.lock().unwrap(),
        [(1, model(EffectOutcome::Succeeded, None, false))]
    );
}

#[tokio::test]
async fn any_concurrent_batch_denial_precedes_every_executor() {
    let trace = trace();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            deny_external_tool: true,
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![
            tool_call("read", "call-read"),
            tool_call("external", "call-external"),
        ])],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definitions = vec![
        tool(
            "read",
            effect(true, false, false),
            ToolConcurrency::Concurrent,
        ),
        tool(
            "external",
            effect(false, true, false),
            ToolConcurrency::Concurrent,
        ),
    ];

    let result =
        run_agent_with_control(&provider, &executor, request(), definitions, control).await;

    assert!(matches!(result, Err(AgentError::EffectAdmissionDenied)));
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *trace.lock().unwrap(),
        ["admit:model", "provider", "settle:1", "admit:tools:2"]
    );
}

#[tokio::test]
async fn malformed_batch_permit_count_refuses_before_dispatch() {
    let trace = trace();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            short_tool_batch: true,
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![
            tool_call("first", "call-1"),
            tool_call("second", "call-2"),
        ])],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let cancellation = CancellationToken::new();
    let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);
    let definitions = vec![
        tool(
            "first",
            effect(true, false, false),
            ToolConcurrency::Concurrent,
        ),
        tool(
            "second",
            effect(false, false, false),
            ToolConcurrency::Concurrent,
        ),
    ];

    let result =
        run_agent_with_control(&provider, &executor, request(), definitions, control).await;

    assert!(matches!(result, Err(AgentError::EffectOwnerFailure(_))));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert!(
        owner
            .settlements
            .lock()
            .unwrap()
            .iter()
            .any(|(_, settlement)| { *settlement == tool_settlement(EffectOutcome::NotStarted) })
    );
}

#[tokio::test]
async fn tool_settlements_follow_input_permits_after_out_of_order_completion() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(
        vec![
            tool_calls(vec![
                tool_call("slow", "call-slow"),
                tool_call("fast", "call-fast"),
            ]),
            answer("done"),
        ],
        trace.clone(),
    );
    let (mut executor, executor_log) = RecordingExecutor::new(trace.clone());
    executor.fail_name = Some("slow".to_owned());
    executor.delay_slow = true;
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);
    let definitions = vec![
        tool(
            "slow",
            effect(false, true, false),
            ToolConcurrency::Concurrent,
        ),
        tool(
            "fast",
            effect(false, true, false),
            ToolConcurrency::Concurrent,
        ),
    ];

    let output = run_agent_with_control(&provider, &executor, request(), definitions, control)
        .await
        .unwrap();

    assert_eq!(output.answer, "done");
    assert_eq!(*executor_log.finished.lock().unwrap(), ["fast", "slow"]);
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 4);
    assert_eq!(settlements[1].0, 2);
    assert_eq!(settlements[1].1, tool_settlement(EffectOutcome::Unknown));
    assert_eq!(settlements[2].0, 3);
    assert_eq!(settlements[2].1, tool_settlement(EffectOutcome::Succeeded));
    assert_eq!(settlements[3].0, 4);
    assert_eq!(
        settlements[3].1,
        model(EffectOutcome::Succeeded, None, false)
    );
}

#[tokio::test]
async fn absent_hook_keeps_the_existing_tool_loop() {
    let trace = trace();
    let provider = ScriptedProvider::new(
        vec![
            tool_calls(vec![tool_call("read", "call-1")]),
            answer("done without hook"),
        ],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new());
    let definition = tool("read", effect(true, false, false), ToolConcurrency::Serial);

    let output = run_agent_with_control(&provider, &executor, request(), vec![definition], control)
        .await
        .unwrap();

    assert_eq!(output.answer, "done without hook");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 1);
}
