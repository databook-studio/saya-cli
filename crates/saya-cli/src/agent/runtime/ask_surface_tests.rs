//! The one-shot `ask` surface's MCP tools: the configured servers are
//! advertised beside the database tools, a call reaches the server under
//! `bypass`, and a headless `ask` refuses with the typed reason. The one
//! `[tools] allow` list governs the family exactly as it governs every other
//! name.

use std::collections::BTreeMap;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolError};
use saya_config::ResolvedTools;
use serde_json::json;

use super::ask_surface;
use crate::agent::tools::DatabaseTools;
use crate::config::runtime::RuntimeConfig;
use crate::mcp::client::support::{Fixture, TIMEOUTS};

/// A resolved runtime whose only MCP server is the fixture.
fn runtime(fixture: &Fixture) -> RuntimeConfig {
    let root = std::env::temp_dir().join(format!("saya-ask-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut runtime = crate::load_with_sources(
        &crate::GlobalOptions::default(),
        &root,
        &root,
        BTreeMap::new(),
    )
    .unwrap();
    runtime.resolved.mcp_servers = fixture.servers(&[], TIMEOUTS);
    runtime
}

fn database() -> Arc<DatabaseTools> {
    Arc::new(DatabaseTools::new(None, 100, true))
}

fn names(definitions: &[saya_agent::ToolDefinition]) -> Vec<&str> {
    definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect()
}

/// `ask` advertises the server's tool after the database surface, and a
/// `bypass` call reaches the server through the surface's own executor.
#[tokio::test]
async fn ask_advertises_and_reaches_the_servers_tools_under_bypass() {
    let fixture = Fixture::new("ask-mcp-bypass");
    let runtime = runtime(&fixture);
    let cancellation = CancellationToken::new();
    let lane = ask_surface::lane(&runtime, ApprovalPolicy::Bypass, &cancellation)
        .await
        .expect("a lane is composed");
    let definitions =
        ask_surface::definitions(true, false, false, Some(&lane), &ResolvedTools::default());
    assert!(
        names(&definitions).contains(&"mcp__fixture__echo"),
        "the surface advertises the server's tool: {:?}",
        names(&definitions)
    );
    assert!(
        names(&definitions)
            .last()
            .is_some_and(|name| name.starts_with("mcp__")),
        "the server's tools are appended last: {:?}",
        names(&definitions)
    );

    let executor = ask_surface::executor(&database(), Some(lane), ResolvedTools::default());
    let result = executor
        .execute("mcp__fixture__echo", json!({ "text": "hi" }))
        .await
        .expect("the surface's executor reaches the server");
    assert!(
        result.to_string().contains("echo"),
        "the server's answer reaches the model: {result}"
    );
}

/// A headless `ask` has no operator, so the call is refused with the typed
/// reason naming the flag that would reach a server.
#[tokio::test]
async fn ask_refuses_a_call_with_no_operator() {
    let fixture = Fixture::new("ask-mcp-headless");
    let runtime = runtime(&fixture);
    let cancellation = CancellationToken::new();
    let lane = ask_surface::lane(&runtime, ApprovalPolicy::Ask, &cancellation)
        .await
        .expect("a lane is composed");
    let executor = ask_surface::executor(&database(), Some(lane), ResolvedTools::default());
    let refused = executor
        .execute("mcp__fixture__echo", json!({ "text": "hi" }))
        .await
        .expect_err("a headless ask refuses the call");
    assert!(
        refused.to_string().contains("--approval-mode bypass"),
        "the refusal names the flag that would reach a server: {refused}"
    );
}

/// The one `[tools] allow` list takes the server's tools away from the
/// advertisement and from the surface's dispatch door.
#[tokio::test]
async fn the_tools_allow_list_removes_the_servers_tools_from_ask() {
    let fixture = Fixture::new("ask-mcp-allow");
    let runtime = runtime(&fixture);
    let cancellation = CancellationToken::new();
    let lane = ask_surface::lane(&runtime, ApprovalPolicy::Bypass, &cancellation)
        .await
        .expect("a lane is composed");
    let allow = ResolvedTools::from_stated(Some(&["read".to_owned()])).unwrap();
    let definitions = ask_surface::definitions(true, false, false, Some(&lane), &allow);
    assert!(
        !names(&definitions).contains(&"mcp__fixture__echo"),
        "the list removes the server's tool: {:?}",
        names(&definitions)
    );

    let executor = ask_surface::executor(&database(), Some(lane), allow);
    let refused = executor
        .execute("mcp__fixture__echo", json!({}))
        .await
        .expect_err("the removed name is unknown at the surface's door");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );
}
