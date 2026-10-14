use super::support::{ControlledScriptProvider, NoTools, Records, successful_script};
use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AllowReadOnlyApproval, CancellationToken, EffectCallKind,
    EffectSettlement, ProviderError, ProviderEvent, ProviderStream, TokenUsage,
    run_agent_with_control,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn strict_mode_refuses_legacy_provider_without_calling_its_old_stream() {
    let provider = LegacyProvider::default();
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let error = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control(&approval, CancellationToken::new()),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentError::UnsupportedPhysicalCoverage));
    assert_eq!(provider.old_calls.load(Ordering::SeqCst), 0);
    assert_eq!(records.attempts_seen.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn logical_only_builder_keeps_legacy_provider_compatibility() {
    let provider = LegacyProvider::default();
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let control = saya_agent::AgentRunControl::new(
        AgentLimits::default(),
        saya_agent::AnsweringLimits {
            max_requests: Some(2),
            max_tool_calls: Some(2),
            max_reported_tokens: Some(10_000),
        },
        &approval,
        &saya_agent::NoopEventSink,
        CancellationToken::new(),
    )
    .with_effect_admission(&records);
    let output =
        run_agent_with_control(&provider, &NoTools, Records::request(), Vec::new(), control)
            .await
            .unwrap();
    assert_eq!(output.answer, "legacy answer");
    assert_eq!(provider.old_calls.load(Ordering::SeqCst), 1);
    assert_eq!(records.attempts_seen.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn outer_retry_is_a_separate_physical_attempt_and_partial_usage_stays_incomplete() {
    let first = vec![Err(ProviderError::Request("fixed transient".into()))];
    let mut second = successful_script();
    let mut usage = TokenUsage::default();
    usage.input_tokens = 7;
    second.insert(0, Ok(ProviderEvent::Usage(usage)));
    let provider = ControlledScriptProvider::new(vec![first, second]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let output = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control(&approval, CancellationToken::new()),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    let physical = records.physical.lock().unwrap();
    assert_eq!(physical.len(), 2);
    assert_eq!(physical[0].1.ordinal(), 0);
    assert_eq!(physical[1].1.ordinal(), 1);
    assert_eq!(physical[0].1.parent(), physical[1].1.parent());
    assert!(
        records
            .settlements
            .lock()
            .unwrap()
            .iter()
            .any(|(_, settlement)| {
                matches!(
                    settlement,
                    EffectSettlement::ModelRequest {
                        usage_complete: false,
                        ..
                    }
                )
            })
    );
}

#[tokio::test]
async fn silent_physical_usage_remains_unknown_in_logical_and_attempt_settlements() {
    let provider = ControlledScriptProvider::new(vec![successful_script()]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control(&approval, CancellationToken::new()),
    )
    .await
    .unwrap();
    let settlements = records.settlements.lock().unwrap();
    assert!(settlements.iter().any(|(_, settlement)| matches!(
        settlement,
        EffectSettlement::ModelRequest {
            reported_usage: None,
            usage_complete: false,
            ..
        }
    )));
    assert!(settlements.iter().any(|(_, settlement)| matches!(
        settlement,
        EffectSettlement::PhysicalAttempt {
            outcome: saya_agent::EffectOutcome::Succeeded,
            reported_usage: None,
            usage_complete: false,
            ..
        }
    )));
}

#[tokio::test]
async fn continuation_and_salvage_keep_distinct_kind_and_logical_parent() {
    let mut usage = TokenUsage::default();
    usage.input_tokens = 2;
    let truncated = vec![
        Ok(ProviderEvent::Usage(usage)),
        Err(ProviderError::output_truncated(
            "partial".into(),
            Vec::new(),
        )),
    ];
    let continuation_provider = ControlledScriptProvider::new(vec![truncated, successful_script()]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let limits = AgentLimits {
        max_continuations: Some(1),
        ..AgentLimits::default()
    };
    let output = run_agent_with_control(
        &continuation_provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control_with_limits(&approval, CancellationToken::new(), limits),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    let attempts = records.physical.lock().unwrap().clone();
    assert_eq!(
        attempts
            .iter()
            .map(|(_, attempt)| attempt.kind())
            .collect::<Vec<_>>(),
        vec![EffectCallKind::Answer, EffectCallKind::Continuation]
    );
    let models = records.models.lock().unwrap().clone();
    assert_eq!(models.len(), 2);
    assert_eq!(attempts[0].1.parent(), models[0]);
    assert_eq!(attempts[1].1.parent(), models[1]);

    let salvage_provider = ControlledScriptProvider::new(vec![successful_script()]);
    let salvage_records = Records::default();
    let limits = AgentLimits {
        max_turns: Some(0),
        ..AgentLimits::default()
    };
    let output = run_agent_with_control(
        &salvage_provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        salvage_records.strict_control_with_limits(&approval, CancellationToken::new(), limits),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    let attempts = salvage_records.physical.lock().unwrap().clone();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].1.kind(), EffectCallKind::Salvage);
    assert_eq!(
        attempts[0].1.parent(),
        salvage_records.models.lock().unwrap()[0]
    );
}

#[derive(Default)]
struct LegacyProvider {
    old_calls: AtomicUsize,
}

#[async_trait]
impl saya_agent::ChatProvider for LegacyProvider {
    fn name(&self) -> &str {
        "n02-legacy"
    }

    async fn complete(
        &self,
        _: saya_agent::ChatRequest,
    ) -> Result<saya_agent::ChatResponse, ProviderError> {
        Err(ProviderError::InvalidResponse)
    }

    async fn stream(
        &self,
        _: saya_agent::ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.old_calls.fetch_add(1, Ordering::SeqCst);
        let events = vec![
            Ok(ProviderEvent::TextDelta("legacy answer".into())),
            Ok(ProviderEvent::Done),
        ];
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
