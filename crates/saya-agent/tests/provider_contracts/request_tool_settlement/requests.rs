use std::sync::atomic::Ordering;

use saya_agent::{
    AgentError, CancellationReason, CancellationToken, DESIGNATE_ANSWER_TOOL, EffectOutcome,
    EffectSettlement, ToolCall, run_agent_with_control,
};

use super::{
    common::{ApproveAll, control, request},
    executor::RecordingExecutor,
    owner::{OwnerOptions, RecordingOwner, model, trace},
    provider::{ScriptedProvider, answer, reported_answer},
};

#[tokio::test]
async fn denied_logical_request_precedes_provider_and_executor() {
    let trace = trace();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            deny_model: true,
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(vec![answer("unused")], trace.clone());
    let (executor, executor_log) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);

    let result = run_agent_with_control(&provider, &executor, request(), Vec::new(), control).await;

    assert!(matches!(result, Err(AgentError::EffectAdmissionDenied)));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(executor_log.calls.load(Ordering::SeqCst), 0);
    assert_eq!(*trace.lock().unwrap(), ["admit:model"]);
    assert!(owner.settlements.lock().unwrap().is_empty());
}

#[tokio::test]
async fn model_admission_precedes_dispatch_and_silent_usage_stays_unknown() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(vec![answer("silent answer")], trace.clone());
    let (executor, _) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);

    let output = run_agent_with_control(&provider, &executor, request(), Vec::new(), control)
        .await
        .unwrap();

    assert_eq!(output.answer, "silent answer");
    assert_eq!(
        *trace.lock().unwrap(),
        ["admit:model", "provider", "settle:1"]
    );
    assert_eq!(
        *owner.settlements.lock().unwrap(),
        [(1, model(EffectOutcome::Succeeded, None, false),)]
    );
}

#[tokio::test]
async fn model_settlement_preserves_reported_usage() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let provider = ScriptedProvider::new(vec![reported_answer("answer", 7, 3)], trace.clone());
    let (executor, _) = RecordingExecutor::new(trace.clone());
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);

    run_agent_with_control(&provider, &executor, request(), Vec::new(), control)
        .await
        .unwrap();

    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 1);
    assert!(matches!(
        &settlements[0].1,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(usage),
            usage_complete: true,
        } if usage.input_tokens == 7 && usage.output_tokens == 3
    ));
}

#[tokio::test]
async fn retry_with_missing_usage_retains_observation_as_incomplete() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let failed_attempt = vec![Err(saya_agent::ProviderError::InvalidResponse)];
    let reported_attempt = reported_answer("retried answer", 7, 3)
        .into_iter()
        .map(Ok)
        .collect();
    let provider =
        ScriptedProvider::new_results(vec![failed_attempt, reported_attempt], trace.clone());
    let (executor, _) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);

    let output = run_agent_with_control(&provider, &executor, request(), Vec::new(), control)
        .await
        .unwrap();

    assert_eq!(output.answer, "retried answer");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let settlements = owner.settlements.lock().unwrap();
    assert_eq!(settlements.len(), 1);
    assert!(matches!(
        &settlements[0].1,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(usage),
            usage_complete: false,
        } if usage.input_tokens == 7 && usage.output_tokens == 3
    ));
}

#[tokio::test]
async fn retry_usage_is_complete_when_each_attempt_reports() {
    let trace = trace();
    let owner = RecordingOwner::new(trace.clone(), OwnerOptions::default());
    let reported_failure = reported_answer("", 2, 1)
        .into_iter()
        .map(Ok)
        .chain(std::iter::once(Err(
            saya_agent::ProviderError::InvalidResponse,
        )))
        .collect();
    let reported_success = reported_answer("retried answer", 7, 3)
        .into_iter()
        .map(Ok)
        .collect();
    let provider =
        ScriptedProvider::new_results(vec![reported_failure, reported_success], trace.clone());
    let (executor, _) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let control = control(&approval, CancellationToken::new()).with_effect_admission(&owner);

    run_agent_with_control(&provider, &executor, request(), Vec::new(), control)
        .await
        .unwrap();

    let settlements = owner.settlements.lock().unwrap();
    assert!(matches!(
        &settlements[0].1,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Succeeded,
            reported_usage: Some(usage),
            usage_complete: true,
        } if usage.input_tokens == 9 && usage.output_tokens == 4
    ));
}

#[tokio::test]
async fn owner_settlement_failure_surfaces_and_stops_the_run() {
    let trace = trace();
    let owner = RecordingOwner::new(
        trace.clone(),
        OwnerOptions {
            fail_settlement: true,
            ..OwnerOptions::default()
        },
    );
    let provider = ScriptedProvider::new(vec![answer("answer")], trace.clone());
    let (executor, _) = RecordingExecutor::new(trace);
    let approval = ApproveAll::default();
    let cancellation = CancellationToken::new();
    let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);

    let result = run_agent_with_control(&provider, &executor, request(), Vec::new(), control).await;

    let error = result.unwrap_err();
    assert!(matches!(error, AgentError::EffectOwnerFailure(_)));
    assert_eq!(
        cancellation.reason(),
        Some(CancellationReason::OwnerFailure)
    );
    assert!(!error.to_string().contains("PRIVATE-OWNER-FAILURE-SENTINEL"));
    assert!(!format!("{error:?}").contains("PRIVATE-OWNER-FAILURE-SENTINEL"));
    assert!(std::error::Error::source(&error).is_some());
}

#[tokio::test]
async fn designated_follow_up_propagates_typed_owner_failures() {
    for owner_failure in [false, true] {
        let trace = trace();
        let cancellation = CancellationToken::new();
        let options = if owner_failure {
            OwnerOptions {
                fail_model_after: Some(1),
                cancel_before_model_failure: Some(cancellation.clone()),
                ..OwnerOptions::default()
            }
        } else {
            OwnerOptions {
                deny_model_after: Some(1),
                ..OwnerOptions::default()
            }
        };
        let owner = RecordingOwner::new(trace.clone(), options);
        let designation = vec![
            saya_agent::ProviderEvent::ToolCalls(vec![ToolCall {
                id: "designation-1".to_owned(),
                name: DESIGNATE_ANSWER_TOOL.to_owned(),
                arguments: serde_json::json!({"sql": "SELECT 1"}),
            }]),
            saya_agent::ProviderEvent::Done,
        ];
        let provider = ScriptedProvider::new(
            vec![designation, answer("the denied follow-up must not run")],
            trace,
        );
        let (executor, _) = RecordingExecutor::new(owner.trace.clone());
        let approval = ApproveAll::default();
        let control = control(&approval, cancellation.clone()).with_effect_admission(&owner);

        let result =
            run_agent_with_control(&provider, &executor, request(), Vec::new(), control).await;

        if owner_failure {
            assert!(matches!(result, Err(AgentError::EffectOwnerFailure(_))));
            assert_eq!(cancellation.reason(), Some(CancellationReason::User));
        } else {
            assert!(matches!(result, Err(AgentError::EffectAdmissionDenied)));
        }
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *owner.settlements.lock().unwrap(),
            [(1, model(EffectOutcome::Succeeded, None, false))]
        );
    }
}
