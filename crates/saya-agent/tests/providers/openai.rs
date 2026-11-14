fn openai(base: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap()
}
