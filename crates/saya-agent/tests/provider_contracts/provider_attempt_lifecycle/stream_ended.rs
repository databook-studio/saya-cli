//! A stream that ends before its terminator without having delivered anything
//! is typed (`StreamEnded`) but is not a known provider answer: the client
//! cannot tell what the provider did, so its attempt and logical request keep
//! settling Unknown, with no usage invented. Typing the cause must not change
//! any settlement rule. The script delivers nothing on purpose — a stream end
//! *after* content is a stream cut, which continues instead of retrying.

use super::support::{ControlledScriptProvider, NoTools, Records, successful_script};
use saya_agent::{
    AgentError, AllowReadOnlyApproval, CancellationToken, EffectOutcome, EffectSettlement,
    ProviderError, ProviderEvent, run_agent_with_control,
};

fn stream_ended_script() -> Vec<Result<ProviderEvent, ProviderError>> {
    vec![Err(ProviderError::StreamEnded)]
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
async fn a_stream_ended_attempt_settles_unknown_without_inventing_usage() {
    let provider = ControlledScriptProvider::new(vec![stream_ended_script(), successful_script()]);
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
    assert_eq!(first.0, EffectOutcome::Unknown);
    assert_eq!(first.1, None, "no usage was reported, so none is invented");
    assert!(
        !first.2,
        "the missing usage keeps token coverage incomplete"
    );
}

#[tokio::test]
async fn a_retry_that_completes_after_a_stream_ended_attempt_settles_the_request_succeeded() {
    let provider = ControlledScriptProvider::new(vec![stream_ended_script(), successful_script()]);
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
    assert_eq!(
        attempt_outcomes(&records),
        vec![EffectOutcome::Unknown, EffectOutcome::Succeeded]
    );
    assert_eq!(request_outcomes(&records), vec![EffectOutcome::Succeeded]);
}

#[tokio::test]
async fn an_exhausted_stream_ended_run_leaves_nine_unknown_attempts_and_an_unknown_request() {
    let provider = ControlledScriptProvider::new(vec![
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
        stream_ended_script(),
    ]);
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
    assert!(
        matches!(error, AgentError::Provider(ProviderError::StreamEnded)),
        "{error:?}"
    );
    assert_eq!(
        attempt_outcomes(&records),
        vec![EffectOutcome::Unknown; 9],
        "all nine attempts stay unknown"
    );
    assert_eq!(
        request_outcomes(&records),
        vec![EffectOutcome::Unknown],
        "no attempt produced a response, so the request stays unknown"
    );
}
