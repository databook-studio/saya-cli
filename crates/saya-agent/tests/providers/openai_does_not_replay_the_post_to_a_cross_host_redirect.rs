/// A cross-host 307 must not replay the OpenAI POST to the redirect target:
/// the body carries the prompt (including database-derived context), and the
/// `Authorization: Bearer` key rides the default policy's replayed request
/// only until reqwest strips it — so the prompt disclosure is the finding
/// this test pins, with the key spared by the strip rather than by policy.
#[tokio::test]
async fn openai_does_not_replay_the_post_to_a_cross_host_redirect() {
    let capture = redirect_capture(
        "/v1/chat/completions",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let provider = openai(capture.origin_base.clone());
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the OpenAI POST to the redirect target"
    );
}
