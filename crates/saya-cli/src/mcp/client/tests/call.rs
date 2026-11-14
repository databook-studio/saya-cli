//! What one call to an MCP tool returns, and what it refuses.
//!
//! The answer is a server's own words, so every test here is about what this
//! client does to them on the way to the model: it bounds them, it scrubs the
//! credentials the server was handed, and it carries the server's own failure
//! flag rather than interpreting it.

use saya_agent::ToolExecutor;
use saya_agent::{ApprovalPolicy, CancellationToken, ToolError};
use serde_json::json;

use super::super::support::{Fixture, TIMEOUTS, lane};

/// A call under `bypass` reaches the server and returns what it answered: the
/// server's name, the server's own tool name, and its text.
#[tokio::test]
async fn an_approved_call_returns_the_servers_answer() {
    let fixture = Fixture::new("call-ok");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let result = lane
        .execute("mcp__fixture__echo", json!({ "text": "hi" }))
        .await
        .expect("the call succeeds");
    assert_eq!(result["server"], "fixture");
    assert_eq!(result["tool"], "echo");
    let content = result["content"].as_str().expect("content is text");
    assert!(content.contains("echo"), "{content}");
    assert!(
        content.contains("hi"),
        "the arguments must reach the server: {content}"
    );
    assert!(result.get("is_error").is_none(), "{result}");
}

/// A name no connected server owns is unsupported, not a run failure: nothing
/// was started and nothing was asked.
#[tokio::test]
async fn an_unknown_tool_is_unsupported() {
    let fixture = Fixture::new("call-unknown");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    assert_eq!(
        lane.execute("mcp__fixture__nope", json!({})).await,
        Err(ToolError::UnsupportedTool)
    );
    assert_eq!(
        lane.execute("shell", json!({})).await,
        Err(ToolError::UnsupportedTool)
    );
}

/// The server's own `isError` is carried, not turned into a transport failure:
/// a tool that ran and reported a refusal is a different thing from a server
/// that could not be reached.
#[tokio::test]
async fn the_servers_error_flag_is_carried() {
    let fixture = Fixture::new("call-fail");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let result = lane
        .execute("mcp__fixture__fail", json!({}))
        .await
        .expect("the call itself succeeds");
    assert_eq!(result["is_error"], true);
    assert!(
        result["content"]
            .as_str()
            .unwrap_or_default()
            .contains("refused"),
        "{result}"
    );
}

/// A value the server was handed in its environment is scrubbed out of
/// anything it echoes back: the server is the one program here that holds a
/// credential, and its answer is what the model reads.
#[tokio::test]
async fn the_servers_own_credential_is_scrubbed_from_the_answer() {
    let fixture = Fixture::new("call-secret");
    let token = "saya-mcp-fixture-token-9f3a";
    let vars = [("FIXTURE_TOKEN", token)];
    let servers = fixture.servers(&vars, TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&vars),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let result = lane
        .execute("mcp__fixture__secret", json!({}))
        .await
        .expect("the call succeeds");
    let content = result["content"].as_str().expect("content is text");
    assert!(
        !content.contains(token),
        "the token reached the model: {content}"
    );
    assert!(content.contains("token is"), "{content}");
}

/// Arguments past the byte cap are refused before anything is written: a
/// server is not asked to parse what this client will not send.
#[tokio::test]
async fn arguments_over_the_byte_cap_are_refused() {
    let fixture = Fixture::new("call-args");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let huge = "x".repeat(super::super::bounds::MAX_ARGUMENT_BYTES + 1);
    let error = lane
        .execute("mcp__fixture__echo", json!({ "text": huge }))
        .await
        .expect_err("oversized arguments are refused");
    let ToolError::Runner(reason) = error else {
        panic!("expected a run failure, got {error:?}");
    };
    assert!(reason.contains("the limit is"), "{reason}");
}

/// A cancelled call is refused rather than left to finish: the turn that asked
/// for it is gone, and its answer has nowhere to go.
#[tokio::test]
async fn a_cancelled_call_is_refused() {
    let fixture = Fixture::new("call-cancelled");
    let servers = fixture.servers(&[], TIMEOUTS);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let lane = super::super::support::lane_with_cancellation(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
        cancellation,
    )
    .expect("a lane is composed");
    let error = lane
        .execute("mcp__fixture__echo", json!({}))
        .await
        .expect_err("a cancelled call is refused");
    let ToolError::Runner(reason) = error else {
        panic!("expected a run failure, got {error:?}");
    };
    assert!(reason.contains("cancelled"), "{reason}");
}
