use super::{
    http::{local_server, provider, read_request},
    support::{NoTools, Records},
};
use saya_agent::{
    AllowReadOnlyApproval, CancellationToken, EffectOutcome, EffectSettlement,
    OpenAiCompatibleProvider, ProviderSettings, run_agent_with_control,
};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn cancellation_before_and_after_send_keeps_distinct_effect_outcomes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let before_server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_ok()
    });
    let before = Arc::new(Records::default());
    let before_token = CancellationToken::new();
    *before.cancel_after_admission.lock().unwrap() = Some(before_token.clone());
    let provider = provider(base, Vec::new());
    let approval = AllowReadOnlyApproval;
    let result = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        before.strict_control(&approval, before_token),
    )
    .await;
    assert!(result.is_err());
    assert!(!before_server.await.unwrap());
    assert!(
        before
            .settlements
            .lock()
            .unwrap()
            .iter()
            .any(|(_, settlement)| {
                matches!(
                    settlement,
                    EffectSettlement::PhysicalAttempt {
                        outcome: EffectOutcome::NotStarted,
                        ..
                    }
                )
            })
    );

    let before_observation = before.observations.lock().unwrap()[0].2.clone();
    assert_eq!(before_observation.dispatch_started_ns(), None);
    assert_eq!(before_observation.outcome(), EffectOutcome::NotStarted);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let after_token = CancellationToken::new();
    let server_token = after_token.clone();
    let after_server = tokio::spawn(async move {
        let Ok(Ok((mut socket, _))) =
            tokio::time::timeout(Duration::from_secs(1), listener.accept()).await
        else {
            return false;
        };
        read_request(&mut socket).await;
        server_token.cancel();
        true
    });
    let after = Arc::new(Records::default());
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(base)).with_retry_delays(Vec::new()),
        Some("redacted-test-key"),
    )
    .unwrap();
    let result = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        after.strict_control(&approval, after_token),
    )
    .await;
    assert!(result.is_err());
    assert!(after_server.await.unwrap());
    assert!(
        after
            .settlements
            .lock()
            .unwrap()
            .iter()
            .any(|(_, settlement)| {
                matches!(
                    settlement,
                    EffectSettlement::PhysicalAttempt {
                        outcome: EffectOutcome::Unknown,
                        ..
                    }
                )
            })
    );
    let after_observation = after.observations.lock().unwrap()[0].2.clone();
    assert_eq!(
        before_observation.clock_domain(),
        after_observation.clock_domain()
    );
    assert_eq!(after_observation.outcome(), EffectOutcome::Unknown);
    assert!(after_observation.finished_ns() >= after_observation.dispatch_started_ns().unwrap());
    assert!(!after_observation.usage_complete());
}

#[tokio::test]
async fn physical_settlement_failure_is_not_hidden_as_provider_success() {
    let records = Arc::new(Records::default());
    records
        .fail_settlement
        .store(1, std::sync::atomic::Ordering::SeqCst);
    let (base_url, server) = local_server(Arc::clone(&records), vec![200]).await;
    let provider = provider(base_url, Vec::new());
    let approval = AllowReadOnlyApproval;
    let result = run_agent_with_control(
        &provider,
        &NoTools,
        Records::request(),
        Vec::new(),
        records.strict_control(&approval, CancellationToken::new()),
    )
    .await;
    assert_eq!(server.await.unwrap(), vec![1]);
    assert!(matches!(
        result,
        Err(saya_agent::AgentError::EffectOwnerFailure(_))
    ));
}
