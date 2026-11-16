use super::support::{HangingControlledProvider, NoTools, Records};
use saya_agent::{
    AgentError, AllowReadOnlyApproval, CancellationToken, EffectOutcome, EffectSettlement,
    ProviderError, run_agent_with_control,
};
use std::time::{Duration, Instant};

/// The request deadline bounds establishment on the controlled path too: a
/// dispatched send that never answers is cut by the loop's own deadline, the
/// physical attempt is settled unknown, and the run continues with its bounded
/// stream-cut recovery instead of waiting out the provider's per-attempt bound.
#[tokio::test]
async fn the_request_deadline_bounds_controlled_establishment() {
    let provider = HangingControlledProvider {
        request_timeout: Duration::from_millis(80),
    };
    let records = Records::default();
    let approval = AllowReadOnlyApproval;
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        run_agent_with_control(
            &provider,
            &NoTools,
            Records::request(),
            Vec::new(),
            records.strict_control(&approval, CancellationToken::new()),
        ),
    )
    .await
    .expect("the request deadline, not the test harness, must bound establishment");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "three 80 ms attempts, not the provider's 60 s bound"
    );
    assert!(
        matches!(
            &result,
            Err(AgentError::Provider(ProviderError::StreamCut { .. }))
        ),
        "the controlled deadline is a typed cut: {result:?}"
    );
    let settlements = records.settlements.lock().unwrap();
    assert!(
        settlements.iter().any(|(_, settlement)| matches!(
            settlement,
            EffectSettlement::PhysicalAttempt {
                outcome: EffectOutcome::Unknown,
                ..
            }
        )),
        "an aborted establishment is an unknown physical effect: {settlements:?}"
    );
    assert!(
        settlements.iter().any(|(_, settlement)| matches!(
            settlement,
            EffectSettlement::ModelRequest {
                outcome: EffectOutcome::Unknown,
                ..
            }
        )),
        "the logical request is unknown too: {settlements:?}"
    );
}
