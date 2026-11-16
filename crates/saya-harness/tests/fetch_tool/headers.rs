use super::*;

// --- caller-stated headers: the first hop, and only the first hop -----------

/// A header set is bounded and validated before it is ever built into a
/// request: an oversized or malformed name or value is refused typed, and the
/// set stops at its own count bound.
#[test]
fn a_header_set_is_bounded_and_validated() {
    let mut headers = saya_harness::fetch::FetchHeaders::new();
    assert!(headers.is_empty());
    headers
        .push("x-subscription-token", "fake-key")
        .expect("a lowercase token name and a visible-ASCII value are accepted");
    assert!(!headers.is_empty());
    assert_eq!(
        headers.iter().collect::<Vec<_>>(),
        vec![("x-subscription-token", "fake-key")]
    );

    assert!(
        headers.push("X-Bad", "v").is_err(),
        "an uppercase name is refused rather than normalized"
    );
    assert!(
        headers.push("bad name", "v").is_err(),
        "a name with a space is refused"
    );
    assert!(
        headers.push("x-ok", "").is_err(),
        "an empty value is refused"
    );
    assert!(
        headers.push("x-ok", "caf\u{e9}").is_err(),
        "a non-ASCII value is refused"
    );
    assert!(
        headers.push("x-ok", &"v".repeat(4096)).is_err(),
        "an oversized value is refused"
    );
    assert!(
        headers.push("x-ok", &"n".repeat(4096)).is_err(),
        "an oversized name is refused"
    );

    let mut full = saya_harness::fetch::FetchHeaders::new();
    for index in 0..saya_harness::fetch::MAX_REQUEST_HEADERS {
        full.push(&format!("x-{index}"), "v")
            .expect("within the count bound");
    }
    assert!(
        full.push("x-over", "v").is_err(),
        "the count bound is enforced, not implied"
    );
}

/// The header set never renders its values: a credential that reached a log
/// through a derived `Debug` would be a credential in a log.
#[test]
fn a_header_set_debug_names_headers_but_never_values() {
    let mut headers = saya_harness::fetch::FetchHeaders::new();
    headers
        .push("x-subscription-token", "s3cret-value")
        .unwrap();
    let rendered = format!("{headers:?}");
    assert!(
        rendered.contains("x-subscription-token"),
        "the name is what a diagnostic needs: {rendered}"
    );
    assert!(
        !rendered.contains("s3cret-value"),
        "the value is a credential and must not render: {rendered}"
    );
}

/// The caller-stated headers ride the first request and no redirect hop: a
/// redirect is a destination the operator never stated, so the credential
/// stays on the request the tool built.
#[tokio::test]
async fn headers_ride_the_first_hop_only() {
    let (site, tool, _pulls) = fixture(
        vec![
            ("/start", redirect("/final")),
            ("/final", ok(vec![b"arrived"])),
        ],
        FetchLimits::for_tool_lane(),
    )
    .await;
    let mut headers = saya_harness::fetch::FetchHeaders::new();
    headers.push("x-subscription-token", "fake-key").unwrap();
    let outcome = tool
        .fetch_bounded_with_headers(
            &format!("https://{HOST}/start"),
            BodyBound::HeadTail { cap: 4096 },
            &headers,
            &CancellationToken::new(),
        )
        .await
        .expect("the redirect is judged and followed");
    assert_eq!(outcome.url, format!("https://{HOST}/final"));

    let seen = site.requests();
    assert_eq!(seen.len(), 2, "both hops were requested: {seen:?}");
    assert_eq!(
        seen[0]
            .headers
            .get("x-subscription-token")
            .map(String::as_str),
        Some("fake-key"),
        "the credential rides the request the tool built"
    );
    assert!(
        !seen[1].headers.contains_key("x-subscription-token"),
        "and never crosses into a hop the operator did not state: {:?}",
        seen[1].headers
    );
}
