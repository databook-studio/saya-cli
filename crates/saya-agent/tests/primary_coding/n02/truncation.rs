//! A complete output-limit truncation is a KNOWN provider answer: the attempt
//! and the logical request must not settle Unknown, because an unknown effect
//! fails the whole job at join. Failures whose outcome the client cannot know
//! (a transient transport error) stay Unknown.

use super::support::{ControlledScriptProvider, NoTools, Records, successful_script};
use saya_agent::{
    AgentLimits, AllowReadOnlyApproval, CancellationToken, EffectOutcome, EffectSettlement,
    ProviderError, ProviderEvent, TokenUsage, run_agent_with_control,
};

fn usage(input: u64, output: u64) -> TokenUsage {
    let mut usage = TokenUsage::default();
    usage.input_tokens = input;
    usage.output_tokens = output;
    usage
}

fn truncated_with_usage() -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![
        Ok(ProviderEvent::Usage(usage(5, 4096))),
        Err(ProviderError::output_truncated(
            "partial".into(),
            Vec::new(),
        )),
    ]
}

fn attempt_outcomes(records: &Records) -> Vec<EffectOutcome> {
    records
        .settlements
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, settlement)| match settlement {
            EffectSettlement::PhysicalAttempt { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect()
}

fn request_outcomes(records: &Records) -> Vec<EffectOutcome> {
    records
        .settlements
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, settlement)| match settlement {
            EffectSettlement::ModelRequest { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_complete_truncation_settles_its_attempt_known_and_the_continuation_proceeds() {
    let provider = ControlledScriptProvider::new(vec![truncated_with_usage(), successful_script()]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let limits = AgentLimits {
        max_continuations: Some(1),
        ..AgentLimits::default()
    };
    let output = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control_with_limits(&approval, CancellationToken::new(), limits),
    )
    .await
    .unwrap();
    assert_eq!(output.answer, "answer");
    // The loop still performs its bounded continuation exactly as before...
    assert_eq!(records.physical.lock().unwrap().len(), 2);
    // ...and neither attempt is an unknown effect.
    assert_eq!(
        attempt_outcomes(&records),
        vec![EffectOutcome::Succeeded, EffectOutcome::Succeeded]
    );
}

#[tokio::test]
async fn a_run_that_ends_on_a_truncation_still_settles_its_request_known() {
    let provider = ControlledScriptProvider::new(vec![truncated_with_usage()]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let limits = AgentLimits {
        max_continuations: Some(0),
        ..AgentLimits::default()
    };
    let result = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control_with_limits(&approval, CancellationToken::new(), limits),
    )
    .await;
    assert!(
        result.is_err(),
        "the truncated answer cannot complete the run"
    );
    assert_eq!(attempt_outcomes(&records), vec![EffectOutcome::Succeeded]);
    assert_eq!(request_outcomes(&records), vec![EffectOutcome::Succeeded]);
}

#[tokio::test]
async fn a_transient_transport_failure_stays_unknown() {
    let first = vec![Err(ProviderError::Request("fixed transient".into()))];
    let provider = ControlledScriptProvider::new(vec![first, successful_script()]);
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
    assert_eq!(
        attempt_outcomes(&records),
        vec![EffectOutcome::Unknown, EffectOutcome::Succeeded],
        "a failure the client cannot classify is never reported as known"
    );
}

#[tokio::test]
async fn a_truncation_without_usage_never_reports_measured_zero_tokens() {
    let silent = vec![Err(ProviderError::output_truncated("p".into(), Vec::new()))];
    let provider = ControlledScriptProvider::new(vec![silent, successful_script()]);
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let limits = AgentLimits {
        max_continuations: Some(1),
        ..AgentLimits::default()
    };
    run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control_with_limits(&approval, CancellationToken::new(), limits),
    )
    .await
    .unwrap();
    let settlements = records.settlements.lock().unwrap();
    let first = settlements
        .iter()
        .find_map(|(_, settlement)| match settlement {
            EffectSettlement::PhysicalAttempt {
                outcome,
                reported_usage,
                usage_complete,
                ..
            } => Some((*outcome, *reported_usage, *usage_complete)),
            _ => None,
        })
        .expect("a physical attempt was settled");
    assert_eq!(first.0, EffectOutcome::Succeeded);
    assert_eq!(first.1, None, "no usage was reported, so none is invented");
    assert!(
        !first.2,
        "the missing usage keeps token coverage incomplete"
    );
}
