//! The egress itself: what the policy refuses before anything connects, what
//! the two bounds do to a page that overruns them, and what a result says it
//! fetched.

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, ToolError};

use super::super::WebFetchTool;
use super::super::bounds::{DEFAULT_BODY_BYTES, MIN_BODY_BYTES, body_cap, max_body_bytes};
use super::super::fixture::{self, CannedNet, HOST, PRIVATE_ADDRESS, bounded_call, call, url};

/// The tool every test here drives: `bypass`, so the mode is never what a
/// refusal is about — the policy and the bounds are.
fn tool(net: Arc<CannedNet>) -> WebFetchTool {
    fixture::tool(net, ApprovalPolicy::Bypass, None).expect("bypass advertises the tool")
}

/// An address literal the policy refuses is refused before any resolution, let
/// alone any connection.
#[tokio::test]
async fn an_address_literal_the_policy_refuses_is_refused() {
    for refused in [
        "https://127.0.0.1/x",
        "https://10.0.0.7/x",
        "https://localhost/x",
    ] {
        let outcome = tool(Arc::new(CannedNet::serving("hello")))
            .call("web_fetch", &call(refused))
            .await;
        assert!(
            matches!(&outcome, Err(ToolError::Fetch(detail)) if detail.contains("fetch refused")),
            "{refused} is refused by the policy: {outcome:?}"
        );
    }
}

/// A hostname that resolves to a refused address is refused too — the
/// resolve-then-deny rule, which is what closes the policy's residual. Nothing
/// reaches the wire.
#[tokio::test]
async fn a_host_that_resolves_to_a_private_address_never_reaches_the_wire() {
    let net = Arc::new(CannedNet::serving("hello").resolving_to(PRIVATE_ADDRESS));
    let outcome = tool(Arc::clone(&net))
        .call("web_fetch", &call(&url("/x")))
        .await;
    assert!(
        matches!(&outcome, Err(ToolError::Fetch(detail)) if detail.contains("fetch refused")),
        "a name that resolves into a refused range is refused: {outcome:?}"
    );
    assert_eq!(
        net.requests(),
        0,
        "the refusal happens before anything connects"
    );
}

/// A scheme other than https is refused: the policy's first gate.
#[tokio::test]
async fn a_non_https_scheme_is_refused() {
    let outcome = tool(Arc::new(CannedNet::serving("hello")))
        .call("web_fetch", &call(&format!("http://{HOST}/x")))
        .await;
    assert!(
        matches!(&outcome, Err(ToolError::Fetch(detail)) if detail.contains("fetch refused")),
        "http is not fetched: {outcome:?}"
    );
}

/// A page larger than the call's bound comes back bounded, with the elision
/// marker and the honest count — never refused for its size.
#[tokio::test]
async fn a_large_page_is_bounded_with_the_marker() {
    let page = "lorem ipsum dolor sit amet ".repeat(20_000);
    let net = Arc::new(CannedNet::serving(page).typed("text/plain"));
    let result = tool(Arc::clone(&net))
        .call("web_fetch", &bounded_call(&url("/big"), MIN_BODY_BYTES))
        .await
        .expect("a page over the bound is bounded, not refused");
    let content = result["content"].as_str().expect("the block is rendered");
    assert!(
        content.contains("bytes elided"),
        "the elision marker is in the text the model reads"
    );
    assert_eq!(result["truncated"], true);
    assert!(
        result["elided_bytes"].as_u64().unwrap() > 0,
        "the count is the text the bound elided: {result}"
    );
    assert!(
        content.len() <= MIN_BODY_BYTES + 512,
        "the rendered block stays inside the bound the call stated: {} bytes",
        content.len()
    );
    assert_eq!(net.requests(), 1, "the page was fetched once");
}

/// A page that fits is passed through whole, with no marker, no `truncated`
/// flag at its default, and a truthful zero.
#[tokio::test]
async fn a_page_that_fits_is_whole() {
    let result = tool(Arc::new(
        CannedNet::serving("a short page").typed("text/plain"),
    ))
    .call("web_fetch", &call(&url("/small")))
    .await
    .unwrap();
    assert!(
        result.get("truncated").is_none(),
        "an uncut page carries no `truncated`: {result}"
    );
    assert_eq!(result["elided_bytes"], 0);
    assert_eq!(result["dropped_bytes"], 0);
    assert!(result["content"].as_str().unwrap().contains("a short page"));
}

/// A result names what it fetched: the final URL, the status, the server's own
/// content type, and the host it came from — the label the untrusted block
/// carries.
#[tokio::test]
async fn a_result_names_the_url_the_status_and_the_content_type() {
    let result = tool(Arc::new(
        CannedNet::serving("<p>hello</p>").typed("text/html; charset=utf-8"),
    ))
    .call("web_fetch", &call(&url("/page")))
    .await
    .unwrap();
    assert_eq!(result["url"], url("/page"));
    assert_eq!(result["status"], 200);
    assert_eq!(result["content_type"], "text/html; charset=utf-8");
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains(&format!("web-fetch {HOST}")),
        "the block is labelled with the host it came from: {result}"
    );
}

/// The bound one call states is the bound the tool enforces: absent means the
/// default, a value inside the range is taken as stated, and one outside it is
/// refused typed rather than clamped — a call never believes it asked for
/// something it did not.
#[test]
fn the_call_states_the_bound_and_cannot_raise_it() {
    assert_eq!(body_cap(&call(&url("/a"))).unwrap(), DEFAULT_BODY_BYTES);
    assert_eq!(body_cap(&bounded_call(&url("/a"), 8192)).unwrap(), 8192);
    assert_eq!(
        body_cap(&bounded_call(&url("/a"), max_body_bytes())).unwrap(),
        max_body_bytes()
    );
    for refused in [0, MIN_BODY_BYTES - 1, max_body_bytes() + 1] {
        assert!(
            body_cap(&bounded_call(&url("/a"), refused)).is_err(),
            "max_bytes={refused} is outside the advertised range"
        );
    }
    assert!(body_cap(&serde_json::json!({"url": url("/a"), "max_bytes": "big"})).is_err());
}

/// A call with no URL, a non-string URL, or one over the byte ceiling is
/// refused before the policy sees it.
#[tokio::test]
async fn a_malformed_call_is_refused_before_the_policy() {
    let net = Arc::new(CannedNet::serving("hello"));
    let tool = tool(Arc::clone(&net));
    for arguments in [
        serde_json::json!({}),
        serde_json::json!({"url": 7}),
        serde_json::json!({"url": ""}),
        serde_json::json!({"url": format!("https://{}/x", "a".repeat(3000))}),
    ] {
        assert!(
            matches!(
                tool.call("web_fetch", &arguments).await,
                Err(ToolError::Fetch(_))
            ),
            "{arguments} is refused before the policy"
        );
    }
    assert_eq!(net.requests(), 0, "none of them reached the wire");
}

/// The tool serves its own name and no other: a name it does not own is
/// unsupported rather than guessed at.
#[tokio::test]
async fn the_tool_serves_only_its_own_name() {
    let tool = tool(Arc::new(CannedNet::serving("hello")));
    assert!(matches!(
        tool.call("http_fetch", &call(&url("/a"))).await,
        Err(ToolError::UnsupportedTool)
    ));
}
