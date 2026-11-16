#[test]
fn every_provider_reports_its_configured_retry_schedule_to_the_loop() {
    let schedule = vec![Duration::from_millis(7), Duration::from_millis(11)];
    let settings = || {
        ProviderSettings::new("test-model", Some("http://127.0.0.1:9".into()))
            .with_retry_delays(schedule.clone())
    };
    let providers: Vec<Box<dyn ChatProvider>> = vec![
        Box::new(OpenAiCompatibleProvider::new(settings(), Some("k")).unwrap()),
        Box::new(saya_agent::AnthropicProvider::new(settings(), Some("k")).unwrap()),
        Box::new(OllamaProvider::new(settings()).unwrap()),
        Box::new(saya_agent::GeminiProvider::new(settings(), Some("k")).unwrap()),
    ];
    for provider in &providers {
        assert_eq!(
            provider.retry_delays(),
            schedule,
            "{} did not hand its configured schedule to the loop",
            provider.name()
        );
    }
}

/// The loop clamps a stream-cut ceiling against the operator's configured
/// value, so every provider must report it — a provider that reported nothing
/// would let the adaptation raise the ceiling past what the operator set.
#[test]
fn every_provider_reports_its_configured_output_ceiling_to_the_loop() {
    let settings = || {
        ProviderSettings::new("test-model", Some("http://127.0.0.1:9".into()))
            .with_max_output_tokens(Some(4321))
    };
    let providers: Vec<Box<dyn ChatProvider>> = vec![
        Box::new(OpenAiCompatibleProvider::new(settings(), Some("k")).unwrap()),
        Box::new(saya_agent::AnthropicProvider::new(settings(), Some("k")).unwrap()),
        Box::new(OllamaProvider::new(settings()).unwrap()),
        Box::new(saya_agent::GeminiProvider::new(settings(), Some("k")).unwrap()),
    ];
    for provider in &providers {
        assert_eq!(
            provider.max_output_tokens(),
            Some(4321),
            "{} did not hand its configured ceiling to the loop",
            provider.name()
        );
    }
}

#[test]
fn a_provider_that_declares_no_schedule_reports_the_default_backoff() {
    assert_eq!(
        FixedStreamProvider(Vec::new()).retry_delays(),
        vec![
            Duration::from_millis(500),
            Duration::from_millis(1000),
            Duration::from_millis(2000),
            Duration::from_millis(4000),
            Duration::from_millis(8000),
            Duration::from_millis(16000),
            Duration::from_millis(30000),
            Duration::from_millis(30000),
        ]
    );
}
