/// A cross-host 307 must not replay the Anthropic POST to the redirect
/// target: reqwest's cross-host strip removes `Authorization` but not the
/// bespoke `x-api-key` header, so both the prompt body and the API key would
/// reach the redirect target under the default policy.
#[tokio::test]
async fn anthropic_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::AnthropicProvider;
    let capture = redirect_capture(
        "/messages",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let provider = AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(capture.origin_base.clone()))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Anthropic POST (prompt body and x-api-key) to the redirect target"
    );
}
