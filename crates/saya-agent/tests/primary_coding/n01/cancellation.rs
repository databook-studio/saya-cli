use std::sync::atomic::Ordering;

use saya_agent::{
    AgentError, AgentLimits, CancellationReason, CancellationToken, EffectOutcome, ToolConcurrency,
    run_agent_with_control, run_agent_with_sink,
};

use super::{
    common::{ApproveAll, CancelBudgetApproval, NOOP, control, effect, request, tool},
    executor::RecordingExecutor,
    owner::{OwnerOptions, RecordingOwner, model, tool as tool_settlement, trace},
    provider::{ScriptedProvider, answer, tool_calls},
};

#[tokio::test]
async fn cancellation_after_model_reservation_but_before_dispatch_settles_not_started() {
    let trace = trace();
    let cancellation = CancellationToken::new();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            cancel_on_model_admission: Some(cancellation.clone()),
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(vec![answer("unused")], trace.clone());
    let (executor, executor_log) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);

    let result = run_agent_with_control(&provider, &executor, request(), Vec::new(), control).await;

    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert_eq!(cancellation.reason(), Some(CancellationReason::User));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *owner.settlements.lock().unwrap(),
        [(1, model(EffectOutcome::NotStarted, None, false))]
    );
}

#[tokio::test]
async fn provider_cancellation_after_dispatch_settles_unknown() {
    let trace = trace();
    let cancellation = CancellationToken::new();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(vec![answer("unobserved")], trace.clone())
        .cancel_on_stream(cancellation.clone());
    let (executor, executor_log) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);

    let result = run_agent_with_control(&provider, &executor, request(), Vec::new(), control).await;

    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *owner.settlements.lock().unwrap(),
        [(1, model(EffectOutcome::Unknown, None, false))]
    );
}

#[tokio::test]
async fn cancellation_after_tool_admission_settles_before_executor_dispatch() {
    let trace = trace();
    let cancellation = CancellationToken::new();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            cancel_on_tool_admission: Some(cancellation.clone()),
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![super::common::tool_call("read", "call-1")])],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);
    let definition = super::common::tool(
        "read",
        super::common::effect(true, false, false),
        saya_agent::ToolConcurrency::Serial,
    );

    let result =
        run_agent_with_control(&provider, &executor, request(), vec![definition], control).await;

    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert_eq!(cancellation.reason(), Some(CancellationReason::User));
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        *owner.settlements.lock().unwrap(),
        [
            (1, model(EffectOutcome::Succeeded, None, false)),
            (2, tool_settlement(EffectOutcome::NotStarted)),
        ]
    );
}

#[tokio::test]
async fn legacy_sink_entrypoint_keeps_uncontrolled_cancellation_as_cancelled() {
    let trace = trace();
    let cancellation = CancellationToken::new();
    let provider = ScriptedProvider::new(
        vec![tool_calls(vec![super::common::tool_call("read", "call-1")])],
        trace.clone(),
    );
    let (executor, executor_log) = RecordingExecutor::new(trace);
    let approval = CancelBudgetApproval(cancellation.clone());
    let definition = tool("read", effect(true, false, true), ToolConcurrency::Serial);

    let result = run_agent_with_sink(
        &provider,
        &executor,
        request(),
        vec![definition],
        AgentLimits::default(),
        &approval,
        &NOOP,
        cancellation.clone(),
    )
    .await;

    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::Budget(
            saya_agent::BudgetDimension::LogicalRequests
        ))
    );
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
}
