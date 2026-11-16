use saya_agent::ToolExecutor;

use super::searxng_answer;
use crate::agent::tools::web::fixture as web;

/// The tool asks the configured endpoint, and only it: the query is encoded
/// into the endpoint's own URL, so the request the wire sees is the one the
/// user configured.
#[tokio::test]
async fn the_request_goes_to_the_configured_endpoint() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute("web_search", serde_json::json!({"query": "rust lifetimes"}))
        .await
        .expect("the search runs");
    assert_eq!(result["results"], 3, "{result}");

    let seen = net.seen();
    assert_eq!(seen.len(), 1, "one request: {seen:?}");
    assert!(
        seen[0].0.starts_with("https://search.example.org/search?"),
        "the endpoint the user configured, not the model's: {}",
        seen[0].0
    );
    assert!(
        seen[0].0.contains("q=rust%20lifetimes"),
        "the query is encoded into that endpoint's URL: {}",
        seen[0].0
    );
    assert!(
        seen[0].0.contains("format=json"),
        "the SearXNG JSON API is what is asked for: {}",
        seen[0].0
    );
}

/// What comes back is the results, as untrusted text: titles, URLs, and
/// snippets reach the model on the same lane a fetched page does.
#[tokio::test]
async fn the_results_arrive_as_a_bounded_untrusted_block() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).typed("application/json"));
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute("web_search", serde_json::json!({"query": "rust lifetimes"}))
        .await
        .expect("the search runs");
    let content = result["content"].as_str().expect("the block is a string");
    assert!(
        content.contains("Validating References"),
        "the title reaches the model: {content}"
    );
    assert!(
        content.contains("https://doc.rust-lang.org/book/ch10-03.html"),
        "and so does the URL: {content}"
    );
    assert!(
        content.contains("Lifetimes ensure that references are valid."),
        "and the snippet: {content}"
    );
    assert!(
        content.contains("web-search"),
        "the block is labelled: {content}"
    );
}

/// `max_results` narrows what the model reads. The bound is a bound on the
/// answer, not on the provider: the request asks for what the call stated.
#[tokio::test]
async fn max_results_bounds_the_answer() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).typed("application/json"));
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute(
            "web_search",
            serde_json::json!({"query": "rust", "max_results": 2}),
        )
        .await
        .expect("the search runs");
    assert_eq!(result["results"], 2, "{result}");
    let content = result["content"].as_str().expect("the block is a string");
    assert!(
        !content.contains("Forum thread"),
        "the third result is not in the answer: {content}"
    );
}

/// The domain filters are applied to the results, and what they removed is
/// reported: a model that sees two of three results is owed the fact that a
/// filter took the third, not left believing the provider found two.
#[tokio::test]
async fn the_domain_filters_are_applied_and_reported() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).typed("application/json"));
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let allowed = tools
        .execute(
            "web_search",
            serde_json::json!({"query": "rust", "allowed_domains": ["rust-lang.org"]}),
        )
        .await
        .expect("the search runs");
    assert_eq!(allowed["results"], 1, "{allowed}");
    assert_eq!(allowed["filtered_out"], 2, "{allowed}");
    let content = allowed["content"].as_str().expect("the block is a string");
    assert!(content.contains("doc.rust-lang.org"), "{content}");
    assert!(!content.contains("blog.example.org"), "{content}");

    let blocked = tools
        .execute(
            "web_search",
            serde_json::json!({"query": "rust", "blocked_domains": ["example.org", "example.net"]}),
        )
        .await
        .expect("the search runs");
    assert_eq!(blocked["results"], 1, "{blocked}");
    assert_eq!(blocked["filtered_out"], 2, "{blocked}");
    let content = blocked["content"].as_str().expect("the block is a string");
    assert!(content.contains("doc.rust-lang.org"), "{content}");
}

/// A subdomain of a listed domain is that domain: an allow list naming
/// `example.org` admits `blog.example.org`, and does not admit
/// `notexample.org` — the match is on the domain boundary, not on a suffix.
#[tokio::test]
async fn a_domain_filter_matches_on_the_domain_boundary() {
    let body = serde_json::json!({
        "results": [
            {"url": "https://blog.example.org/a", "title": "A", "content": "a"},
            {"url": "https://notexample.org/b", "title": "B", "content": "b"}
        ]
    })
    .to_string();
    let net = std::sync::Arc::new(web::CannedNet::serving(body).typed("application/json"));
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute(
            "web_search",
            serde_json::json!({"query": "x", "allowed_domains": ["example.org"]}),
        )
        .await
        .expect("the search runs");
    assert_eq!(result["results"], 1, "{result}");
    let content = result["content"].as_str().expect("the block is a string");
    assert!(content.contains("blog.example.org"), "{content}");
    assert!(!content.contains("notexample.org"), "{content}");
}

/// Brave is asked in its own shape, and its own response shape is read.
#[tokio::test]
async fn the_brave_backend_asks_and_reads_in_its_own_shape() {
    let body = serde_json::json!({
        "web": {"results": [
            {"url": "https://a.example/1", "title": "First", "description": "one"}
        ]}
    })
    .to_string();
    let net = std::sync::Arc::new(web::CannedNet::serving(body).typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::brave_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute(
            "web_search",
            serde_json::json!({"query": "rust", "max_results": 3}),
        )
        .await
        .expect("the search runs");
    assert_eq!(result["results"], 1, "{result}");
    let content = result["content"].as_str().expect("the block is a string");
    assert!(
        content.contains("one"),
        "the description is the snippet: {content}"
    );

    let seen = net.seen();
    assert!(
        seen[0]
            .0
            .starts_with("https://api.search.brave.com/res/v1/web/search?"),
        "{}",
        seen[0].0
    );
    assert!(
        seen[0].0.contains("count=3"),
        "the call's own bound is what the provider is asked for: {}",
        seen[0].0
    );
}
