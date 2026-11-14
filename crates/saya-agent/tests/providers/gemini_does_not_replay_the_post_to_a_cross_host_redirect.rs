/// A cross-host 307 must not replay the Gemini POST to the redirect target:
/// reqwest's cross-host strip removes `Authorization` but not the bespoke
/// `x-goog-api-key` header, so both the prompt body and the API key would
/// reach the redirect target under the default policy.
#[tokio::test]
async fn gemini_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::GeminiProvider;
    let capture = redirect_capture(
        "/v1beta/models/test-model:generateContent",
        r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#,
    );
    let origin_base = capture.origin_base.clone();
    let provider = GeminiProvider::new(
        ProviderSettings::new("test-model", Some(format!("{origin_base}/v1beta")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Gemini POST (prompt body and x-goog-api-key) to the redirect target"
    );
}
