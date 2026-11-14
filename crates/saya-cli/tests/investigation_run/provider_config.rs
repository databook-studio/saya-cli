/// A provider config that cannot be reached: any provider call fails fast.
fn provider_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("SAYA_PROVIDER", "openai_compatible"),
        ("SAYA_MODEL", "mock-model"),
        ("SAYA_PROVIDER_BASE_URL", "http://127.0.0.1:9/v1"),
        ("SAYA_API_KEY", "mock-secret"),
    ]
}
