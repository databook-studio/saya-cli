use saya_agent::ToolExecutor;

use crate::agent::tools::web::fixture as web;

const ANSWER: &str =
    r#"{"web":{"results":[{"url":"https://a.example/1","title":"First","description":"one"}]}}"#;

/// The configured credential reaches the wire and nowhere else: not the tool
/// result the model reads, not the refusal text, not the definition the model
/// is shown, and not the block either.
#[tokio::test]
async fn the_credential_reaches_the_wire_and_nowhere_else() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::brave_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    let result = tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect("the search runs");
    let rendered = result.to_string();
    assert!(
        !rendered.contains(web::TEST_KEY),
        "the result the model reads carries no credential: {rendered}"
    );
    for definition in tools.definitions() {
        let schema = serde_json::to_string(&definition).expect("the definition serializes");
        assert!(
            !schema.contains(web::TEST_KEY),
            "the advertisement carries no credential: {schema}"
        );
    }

    let seen = net.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(
        seen[0].1.get("x-subscription-token").map(String::as_str),
        Some(web::TEST_KEY),
        "and it did reach the wire it authenticates: {seen:?}"
    );
}

/// A credential reference that cannot be resolved fails the call with a
/// message naming the *reference*, never the value: a misconfigured key is a
/// fact about the user's environment, not a secret to print.
#[tokio::test]
async fn an_unresolvable_credential_fails_the_call_without_a_value() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let tools = web::web_tools_with_resolver(
        net.clone(),
        Some(web::brave_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
        web::resolver_without_the_key(),
    )
    .expect("bypass composes a lane");

    let error = tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect_err("an unresolvable credential is a failure");
    let text = error.to_string();
    assert!(
        text.contains("SAYA_BRAVE_API_KEY"),
        "the refusal names the reference to fix: {text}"
    );
    assert_eq!(net.requests(), 0, "and nothing was sent unauthenticated");
}

/// A backend that needs no credential sends none — the header set is empty
/// on the wire, not a placeholder.
#[tokio::test]
async fn a_searxng_request_carries_no_credential_header() {
    let net =
        std::sync::Arc::new(web::CannedNet::serving("{\"results\":[]}").typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");

    tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect("the search runs");
    let seen = net.seen();
    assert!(
        seen[0].1.keys().all(|name| !name.contains("token")),
        "no credential header is invented for a backend that has none: {seen:?}"
    );
}
