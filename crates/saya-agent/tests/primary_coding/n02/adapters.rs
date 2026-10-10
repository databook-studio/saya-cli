use super::support::{NoTools, Records};
use saya_agent::{
    AgentLimits, AgentRunControl, AllowReadOnlyApproval, AnthropicProvider, CancellationToken,
    EffectOutcome, EffectSettlement, GeminiProvider, OllamaProvider, OpenAiCompatibleProvider,
    ProviderSettings, run_agent_with_control,
};
use std::time::Duration;

#[tokio::test]
async fn every_builtin_refuses_a_denied_send_before_the_server_accepts_it() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_ok()
    });
    let records = Records::default();
    records.denied.store(1, std::sync::atomic::Ordering::SeqCst);
    let approval = AllowReadOnlyApproval;
    let providers: Vec<Box<dyn saya_agent::ChatProvider>> = vec![
        Box::new(
            OpenAiCompatibleProvider::new(settings(&base), Some("redacted-test-key")).unwrap(),
        ),
        Box::new(AnthropicProvider::new(settings(&base), Some("redacted-test-key")).unwrap()),
        Box::new(GeminiProvider::new(settings(&base), Some("redacted-test-key")).unwrap()),
        Box::new(OllamaProvider::new(settings(&base)).unwrap()),
    ];
    for provider in providers {
        let control = AgentRunControl::new(
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
        .with_physical_effect_admission(&records);
        let error = run_agent_with_control(
            &*provider,
            &NoTools,
            super::support::Records::request(),
            Vec::new(),
            control,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            saya_agent::AgentError::EffectAdmissionDenied
        ));
    }
    assert!(!server.await.unwrap(), "no request may reach the endpoint");
    assert_eq!(
        records
            .attempts_seen
            .load(std::sync::atomic::Ordering::SeqCst),
        4
    );
    let model_permits = records.models.lock().unwrap().clone();
    assert_eq!(model_permits.len(), 4);
    let model_settlements = records
        .settlements
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, settlement)| match settlement {
            EffectSettlement::ModelRequest { outcome, .. } => Some(*outcome),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(model_settlements, vec![EffectOutcome::NotStarted; 4]);
}

#[test]
fn all_builtin_constructors_refuse_unbounded_retry_settings() {
    let settings = || {
        let mut value = ProviderSettings::new("m", Some("http://127.0.0.1".into()));
        value.retry_delays = vec![Duration::ZERO; 4];
        value
    };
    assert!(OpenAiCompatibleProvider::new(settings(), None).is_err());
    assert!(AnthropicProvider::new(settings(), None).is_err());
    assert!(GeminiProvider::new(settings(), None).is_err());
    assert!(OllamaProvider::new(settings()).is_err());

    let mut too_long = ProviderSettings::new("m", None);
    too_long.retry_delays = vec![Duration::from_secs(31)];
    assert!(OllamaProvider::new(too_long).is_err());
}

fn settings(base_url: &str) -> ProviderSettings {
    let mut value = ProviderSettings::new("test-model", Some(base_url.to_owned()));
    value.retry_delays.clear();
    value.timeout = Duration::from_millis(200);
    value
}
