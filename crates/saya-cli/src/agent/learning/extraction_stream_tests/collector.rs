use super::*;

/// The measured failure shape: a model ignoring JSON mode writes prose until
/// its output ceiling truncates it. The first visible character decides, so
/// the collector returns within 1s with an error naming non-JSON instead of
/// waiting out a stream that can never parse.
#[tokio::test]
async fn prose_first_reply_fails_fast() {
    let provider = scripted(vec![delta(PROSE)], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&provider, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    let error = collected.expect_err("a prose reply must be refused");
    assert!(
        error.to_string().contains("not JSON"),
        "the error must name non-JSON, got: {error}"
    );
}

#[tokio::test]
async fn the_stop_cancels_the_provider_token() {
    let provider = scripted(vec![delta("Sure, let me think about this database.")], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&provider, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    let _ = collected.expect_err("a prose reply must be refused");
    let token = provider
        .captured
        .lock()
        .unwrap()
        .clone()
        .expect("the stream was handed a cancellation token");
    assert!(
        token.is_cancelled(),
        "the stop path must cancel the token handed to the provider"
    );
}

#[tokio::test]
async fn reasoning_before_json_is_accepted() {
    let provider = scripted(
        vec![
            reasoning("thinking"),
            reasoning("thinking"),
            reasoning("thinking"),
            delta(r#"{"proposals": []}"#),
            done(),
        ],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("reasoning before JSON must not stop the collector");
    assert_eq!(reply.content, r#"{"proposals": []}"#);
}

#[tokio::test]
async fn json_split_across_deltas_is_accepted() {
    let provider = scripted(
        vec![
            delta("  \n"),
            delta("{"),
            delta(r#""proposals": []}"#),
            done(),
        ],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("whitespace before the JSON must not stop the collector");
    assert_eq!(reply.content, "  \n{\"proposals\": []}");
}

#[tokio::test]
async fn fenced_json_is_accepted() {
    let provider = scripted(
        vec![delta("```json\n{\"proposals\": []}\n```"), done()],
        false,
    );
    let reply = collect_extraction(&provider, request())
        .await
        .expect("a backtick-led reply may be fenced JSON");
    assert_eq!(reply.content, "```json\n{\"proposals\": []}\n```");
}

#[tokio::test]
async fn usage_survives_both_paths() {
    let reported = TokenUsage::new(11, 7);

    let success = scripted(vec![usage(), delta(r#"{"proposals": []}"#), done()], false);
    let reply = collect_extraction(&success, request())
        .await
        .expect("a JSON reply succeeds");
    assert_eq!(
        reply.usage,
        Some(reported),
        "usage reaches the success path"
    );

    let stopped = scripted(vec![usage(), delta(PROSE)], true);
    let collected = tokio::time::timeout(
        Duration::from_secs(1),
        collect_extraction(&stopped, request()),
    )
    .await
    .expect("the collector must return within 1s, not wait out the stream");
    match collected.expect_err("a prose reply must be refused") {
        ExtractionStreamError::NotJson { usage } => {
            assert_eq!(
                usage,
                Some(reported),
                "usage reported before the stop is kept"
            );
        }
        other => panic!("expected NotJson, got {other:?}"),
    }
}

#[tokio::test]
async fn oversize_stream_is_rejected() {
    let oversized = format!("{{{}", "A".repeat(MAX_STREAM_BYTES));
    let provider = scripted(vec![delta(&oversized)], false);
    let error = collect_extraction(&provider, request())
        .await
        .expect_err("a stream past the byte bound must be rejected");
    match error {
        ExtractionStreamError::Provider(ProviderError::Request(message)) => {
            assert!(
                message.contains("size limit"),
                "size-limit error, got: {message}"
            );
        }
        other => panic!("expected the provider size-limit error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_mid_stream_error_is_returned_as_the_error() {
    let provider = scripted(
        vec![
            delta("{"),
            Err(ProviderError::Request("mid-stream failure".into())),
        ],
        false,
    );
    let error = collect_extraction(&provider, request())
        .await
        .expect_err("a stream error must be returned as the error");
    match error {
        ExtractionStreamError::Provider(ProviderError::Request(message)) => {
            assert_eq!(message, "mid-stream failure");
        }
        other => panic!("expected the provider error, got {other:?}"),
    }
}
