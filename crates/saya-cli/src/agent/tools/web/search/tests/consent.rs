use saya_agent::{ApprovalDecider, ToolExecutor};

use crate::agent::tools::web::fixture as web;

/// A searxng answer the fixture's canned wire serves, so a consented call
/// completes and the ask count can be read.
const ANSWER: &str =
    r#"{"results":[{"url":"https://a.example/1","title":"First","content":"one"}]}"#;

/// `ask` puts each search to the operator, once per call — the same rule
/// `web_fetch` follows, and for the same reason: the query is the user's, and
/// where it goes is the operator's decision. The decision is the loop's
/// (`ApprovalDecider::approve`, before the call); the call itself then runs
/// under the mode it was composed with.
#[tokio::test]
async fn ask_puts_every_search_to_the_operator() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let operator = web::RecordingOperator::new(true);
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Ask,
        Some(operator.clone()),
    )
    .expect("ask composes a lane");
    let definition = web::definition_of(&tools, "web_search");

    let first = serde_json::json!({"query": "rust lifetimes"});
    assert!(
        tools.approve(&definition, &first).await,
        "the operator approved"
    );
    let result = tools
        .execute("web_search", first)
        .await
        .expect("the approved search runs");
    assert_eq!(result["results"], 1, "{result}");
    assert_eq!(operator.calls(), 1, "one prompt for one call");

    let second = serde_json::json!({"query": "rust traits"});
    assert!(tools.approve(&definition, &second).await);
    tools
        .execute("web_search", second)
        .await
        .expect("the approved search runs");
    assert_eq!(operator.calls(), 2, "a second call is a second question");
}

/// A refusal is a normal denial: the call does not run, and no request
/// reaches the wire.
#[tokio::test]
async fn a_refused_search_never_reaches_the_wire() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let operator = web::RecordingOperator::new(false);
    let tools = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Ask,
        Some(operator.clone()),
    )
    .expect("ask composes a lane");
    let definition = web::definition_of(&tools, "web_search");

    assert!(
        !tools
            .approve(&definition, &serde_json::json!({"query": "rust"}))
            .await,
        "a refusal is a denial, not a result"
    );
    assert_eq!(operator.calls(), 1, "the operator was asked");
    assert_eq!(net.requests(), 0, "and nothing was sent");
}

/// A headless `ask` job has nobody to answer, so the call is refused with the
/// reason naming the flag that would reach the network — the same refusal
/// `web_fetch` gives, worded for a search.
#[tokio::test]
async fn a_headless_ask_is_refused_naming_the_flag() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let tools = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Ask,
        None,
    )
    .expect("the lane is composed; the call is what refuses");

    let definition = tools
        .definitions()
        .into_iter()
        .find(|definition| definition.name == "web_search")
        .expect("the tool is advertised");
    assert!(
        !tools
            .approve(&definition, &serde_json::json!({"query": "rust"}))
            .await,
        "a headless ask denies"
    );
    let detail = tools
        .refusal_detail(&definition, &serde_json::json!({"query": "rust"}))
        .expect("a denial always has a reason to give");
    assert!(
        detail.contains("--approval-mode bypass"),
        "the refusal names the flag that reaches the network: {detail}"
    );

    let outcome = tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await;
    assert!(outcome.is_err(), "{outcome:?}");
    assert_eq!(net.requests(), 0, "and nothing was sent");
}

/// `bypass` searches without asking: the mode is the operator's standing
/// answer, so no prompt is raised and none is needed.
#[tokio::test]
async fn bypass_searches_without_asking() {
    let net = std::sync::Arc::new(web::CannedNet::serving(ANSWER).typed("application/json"));
    let operator = web::RecordingOperator::new(false);
    let tools = web::web_tools(
        net,
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        Some(operator.clone()),
    )
    .expect("bypass composes a lane");

    tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect("bypass searches");
    assert_eq!(operator.calls(), 0, "bypass asks nobody");
}
