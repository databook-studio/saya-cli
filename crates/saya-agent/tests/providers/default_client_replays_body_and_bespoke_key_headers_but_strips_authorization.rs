/// Probe establishing what reqwest's default redirect handling does with the
/// headers and body these providers send: `Authorization` (OpenAI's
/// `bearer_auth`) is stripped on a cross-host hop, but the bespoke key
/// headers (`x-api-key`, `x-goog-api-key`) are replayed untouched — and a
/// 307 replays the POST body carrying the prompt. Read against reqwest
/// 0.12's `redirect::remove_sensitive_headers`, which removes exactly
/// `authorization`, `cookie`, `cookie2`, `proxy-authorization`, and
/// `www-authenticate`.
#[tokio::test]
async fn default_client_replays_body_and_bespoke_key_headers_but_strips_authorization() {
    let capture = redirect_capture(
        "/v1/chat/completions",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let client = reqwest::Client::builder().build().unwrap();
    let url = format!("{}/v1/chat/completions", capture.origin_base);
    let _ = client
        .post(&url)
        .header("authorization", "Bearer secret-sentinel")
        .header("x-api-key", "secret-sentinel")
        .header("x-goog-api-key", "secret-sentinel")
        .body("{\"messages\":[{\"content\":\"database-derived context\"}]}")
        .send()
        .await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap();
    assert_eq!(
        hits.len(),
        1,
        "the default client must follow the 307 for this probe to say anything"
    );
    let replayed = &hits[0];
    assert!(
        replayed.contains("database-derived context"),
        "a 307 replays the POST body to the redirect target: {replayed}"
    );
    assert!(
        !replayed.to_ascii_lowercase().contains("authorization:"),
        "Authorization is stripped on a cross-host hop: {replayed}"
    );
    assert!(
        replayed.contains("x-api-key: secret-sentinel"),
        "the bespoke x-api-key header travels to the redirect target: {replayed}"
    );
    assert!(
        replayed.contains("x-goog-api-key: secret-sentinel"),
        "the bespoke x-goog-api-key header travels to the redirect target: {replayed}"
    );
}
