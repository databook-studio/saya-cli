use super::*;

/// The pre-bound: a body over the wired bound is the tool's own typed
/// `BodyTooLarge` — a failure, never a short success.
#[tokio::test]
async fn a_body_over_the_wired_bound_fails_typed() {
    let (_root, workspace) = workspace("over-bound");
    let (net, _calls) = StaticNet::serve(&vec![b'a'; lane_bound() + 1], 4_096);
    let tools = adapter(
        net,
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let error = tools
        .execute("http_fetch", serde_json::json!({"url": url("/doc")}))
        .await
        .expect_err("over the wired bound");
    let ToolError::Fetch(detail) = error else {
        panic!("the fetch lane's typed error, got: {error:?}")
    };
    assert!(
        detail.contains(&format!("exceeded {} bytes", lane_bound())),
        "the typed bound names its limit: {detail}"
    );
}

/// A body at the bound arrives closed, with the closing sentinel in the
/// tool message and no loop truncation marker — the pre-bound holds.
#[tokio::test]
async fn a_body_at_the_bound_arrives_closed_without_loop_truncation() {
    let (_root, workspace) = workspace("at-bound");
    let (net, _calls) = StaticNet::serve(&vec![b'a'; lane_bound()], 4_096);
    let tools = adapter(
        net,
        workspace,
        DownloadBudget::default(),
        FetchLimits::for_tool_lane(),
    );
    let value = tools
        .execute("http_fetch", serde_json::json!({"url": url("/doc")}))
        .await
        .expect("a body at the bound fits the lane");
    let serialized = value.to_string();
    let cap = FetchLimits::tool_lane_cap();
    assert!(
        serialized.len() <= cap,
        "the whole result fits the tool-message cap: {} vs {cap}",
        serialized.len()
    );
    let content = value["content"].as_str().expect("the envelope's content");
    assert!(
        content.contains(CLOSE),
        "the tool message contains the closing sentinel: {}",
        &content[..80]
    );
    assert!(
        !serialized.contains("…[truncated: tool result exceeded"),
        "the loop's bounded_json truncation never fires on a fetch result"
    );
}
