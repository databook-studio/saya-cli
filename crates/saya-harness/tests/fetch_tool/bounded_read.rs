use super::*;

// --- the caller-stated body bound: both ends, or the whole body -------------

/// A body over the cap comes back as its head and its tail inside the cap,
/// with the elision marker naming exactly what was dropped — the `web_fetch`
/// rule, where a page longer than the bound is shortened rather than refused.
/// The report accounts for every byte the body held.
#[tokio::test]
async fn a_body_over_the_cap_comes_back_bounded_with_the_marker() {
    let head = "H".repeat(4096);
    let tail = "T".repeat(4096);
    let (site, tool, _pulls) = fixture(
        vec![("/long", ok(vec![head.as_bytes(), tail.as_bytes()]))],
        FetchLimits::default(),
    )
    .await;
    let outcome = tool
        .fetch_bounded_cancellable(
            "https://corpus.example.org/long",
            BodyBound::HeadTail { cap: 2048 },
            &CancellationToken::new(),
        )
        .await
        .expect("a body over the cap is bounded, never refused");
    assert_eq!(outcome.total_bytes, 8192);
    assert!(outcome.truncated, "the outcome says the body was cut");
    assert!(outcome.block.truncated, "and so does the block");
    assert!(outcome.block.body.starts_with("HHHH"), "the head is kept");
    assert!(outcome.block.body.ends_with("TTTT"), "the tail is kept");
    assert!(
        outcome
            .block
            .body
            .contains(&elision_marker(outcome.dropped_bytes)),
        "the marker names the exact count: {}",
        outcome.block.body
    );
    let marker = elision_marker(outcome.dropped_bytes).len();
    assert_eq!(
        (outcome.block.body.len() - marker) as u64 + outcome.dropped_bytes,
        outcome.total_bytes,
        "retained + dropped is every byte the body held"
    );
    assert!(
        outcome.block.body.len() <= 2048,
        "the render stays inside the cap the caller stated: {} bytes",
        outcome.block.body.len()
    );
    assert_eq!(site.served(), vec!["/long".to_owned()]);
}

/// A body that fits the cap is the whole body, byte for byte: no marker, and
/// a truthful zero.
#[tokio::test]
async fn a_body_that_fits_is_whole() {
    let (site, tool, _pulls) = fixture(
        vec![("/short", ok(vec![b"a short body"]))],
        FetchLimits::default(),
    )
    .await;
    let outcome = tool
        .fetch_bounded_cancellable(
            "https://corpus.example.org/short",
            BodyBound::HeadTail { cap: 2048 },
            &CancellationToken::new(),
        )
        .await
        .expect("a body inside the cap is the whole body");
    assert_eq!(outcome.block.body, "a short body");
    assert!(!outcome.truncated);
    assert!(!outcome.block.truncated);
    assert_eq!(outcome.dropped_bytes, 0);
    assert_eq!(outcome.total_bytes, 12);
    assert_eq!(site.served(), vec!["/short".to_owned()]);
}

/// The other bound keeps its own meaning: under [`BodyBound::Whole`] — what
/// [`HttpFetchTool::fetch`] states — a body over the byte limit is still a
/// typed refusal, because that reader's destination is one bounded turn it
/// must fill or fail. Stating a bound never silently changes the other's rule.
#[tokio::test]
async fn the_whole_bound_still_refuses_what_it_cannot_fit() {
    let big = vec![b'x'; 32 * 1024];
    let (_site, tool, _pulls) = fixture(
        vec![("/big", ok(vec![&big]))],
        FetchLimits {
            max_total_bytes: 16 * 1024,
            ..FetchLimits::default()
        },
    )
    .await;
    let error = tool
        .fetch_bounded_cancellable(
            "https://corpus.example.org/big",
            BodyBound::Whole,
            &CancellationToken::new(),
        )
        .await
        .expect_err("the whole bound refuses an overrun");
    assert!(
        matches!(
            error,
            FetchToolError::BodyTooLarge { limit } if limit == 16 * 1024
        ),
        "{error:?}"
    );
}
