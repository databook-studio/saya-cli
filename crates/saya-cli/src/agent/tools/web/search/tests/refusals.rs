//! What a search refuses, and what it reports as a failure rather than as a
//! result. Every one of these is a case where an answer that *looked* fine
//! would have been a lie: an argument silently clamped, a redirect quietly
//! followed, an unreadable answer read as "nothing matched".

use saya_agent::ToolExecutor;

use super::super::bounds::{MAX_QUERY_BYTES, MAX_RESULTS};
use super::searxng_answer;
use crate::agent::tools::web::fixture as web;

/// Every argument is bounded before a request exists: a call that states an
/// out-of-range value is refused typed rather than clamped, so the model
/// never believes it asked for something it did not.
#[tokio::test]
async fn out_of_range_arguments_are_refused_typed() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let long_query = "q".repeat(MAX_QUERY_BYTES + 1);
    for call in [
        serde_json::json!({"query": ""}),
        serde_json::json!({"query": long_query}),
        serde_json::json!({"query": "x", "max_results": 0}),
        serde_json::json!({"query": "x", "max_results": MAX_RESULTS + 1}),
        serde_json::json!({"query": "x", "max_results": "three"}),
        serde_json::json!({"query": "x", "allowed_domains": ["https://example.org/a"]}),
        serde_json::json!({"query": "x", "allowed_domains": ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]}),
        serde_json::json!({"query": "x", "blocked_domains": "example.org"}),
        serde_json::json!({"query": 7}),
        serde_json::json!({"query": "x", "unknown": true}),
    ] {
        assert!(
            tools.execute("web_search", call.clone()).await.is_err(),
            "{call} must be refused"
        );
    }
    assert_eq!(net.requests(), 0, "no refused call reached the wire");
}

/// A search endpoint that answers with a redirect is refused: the search
/// request goes only to the endpoint the user configured, so a hop to
/// somewhere else is not followed — the query does not leave for a host
/// nobody stated.
#[tokio::test]
async fn a_redirecting_endpoint_is_refused() {
    let net = std::sync::Arc::new(web::CannedNet::serving(searxng_answer()).redirecting());
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let error = tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect_err("a redirect is refused");
    let text = error.to_string();
    assert!(
        text.contains("redirect"),
        "the refusal says what happened: {text}"
    );
}

/// An answer that is not the JSON the backend documents is a typed failure,
/// never a silent empty result: the model is owed the difference between "no
/// results" and "the endpoint did not answer with results".
#[tokio::test]
async fn an_unreadable_answer_is_a_typed_failure() {
    for body in ["not json at all", "{\"unexpected\": true}", ""] {
        let net = std::sync::Arc::new(web::CannedNet::serving(body).typed("application/json"));
        let tools = web::web_tools(
            net,
            Some(web::searxng_backend()),
            saya_agent::ApprovalPolicy::Bypass,
            None,
        )
        .expect("bypass composes a lane");
        let error = tools
            .execute("web_search", serde_json::json!({"query": "rust"}))
            .await
            .expect_err("an unreadable answer is a failure");
        assert!(error.to_string().contains("search"), "{body:?}: {error}");
    }
}

/// An answer with no results at all is a success, not a failure: the backend
/// answered, and the answer is that nothing matched.
#[tokio::test]
async fn an_empty_answer_is_a_result_not_a_failure() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving("{\"results\":[]}").typed("application/json"));
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
            serde_json::json!({"query": "nothing matches this"}),
        )
        .await
        .expect("an empty answer is an answer");
    assert_eq!(result["results"], 0, "{result}");
    let content = result["content"].as_str().expect("the block is a string");
    assert!(
        content.to_lowercase().contains("no results"),
        "the model is told plainly: {content}"
    );
}
