use saya_agent::ApprovalPolicy;
use saya_config::ResolvedTools;

use crate::mcp::client::support::{Fixture, TIMEOUTS, lane as compose_lane};

/// A run step advertises the configured servers' tools last, and a call is
/// refused headless: the run has no operator, so `ask` refuses with the typed
/// reason naming the flag that would reach a server. The same lane's `bypass`
/// mode is what reaches one.
#[tokio::test]
async fn a_step_advertises_the_servers_tools_and_refuses_a_headless_call() {
    let fixture = Fixture::new("run-mcp");
    let servers = fixture.servers(&[], TIMEOUTS);
    let resolver = fixture.resolver(&[]);
    let lane = compose_lane(&servers, &resolver, ApprovalPolicy::Ask, None)
        .expect("a lane is composed");
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace("mcp-member");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            mcp: Some(&lane),
            workspace: &ws,
            allow_query_data: true,
            tools: &ResolvedTools::default(),
            hooks: None,
        },
        &[step("use the server", Capabilities::default())],
    );

    let names = names(&built, 0);
    assert!(
        names.contains(&"mcp__fixture__echo".to_string()),
        "the step advertises the server's tool: {names:?}"
    );
    assert!(
        names.last().is_some_and(|name| name.starts_with("mcp__")),
        "the new family is appended last: {names:?}"
    );
    let refused = built[0]
        .executor
        .execute("mcp__fixture__echo", serde_json::json!({ "text": "hi" }))
        .await
        .expect_err("a headless run step refuses the call");
    assert!(
        refused.to_string().contains("--approval-mode bypass"),
        "the refusal names the flag that would reach a server: {refused}"
    );
}

/// The run's `[tools] allow` list takes the server's tools away from the
/// advertisement and from the step's dispatch door, in either spelling.
#[tokio::test]
async fn the_tools_allow_list_removes_the_servers_tools_from_a_step() {
    let fixture = Fixture::new("run-mcp-allow");
    let servers = fixture.servers(&[], TIMEOUTS);
    let resolver = fixture.resolver(&[]);
    let lane = compose_lane(&servers, &resolver, ApprovalPolicy::Ask, None)
        .expect("a lane is composed");
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace("mcp-member-allow");
    let allow = ResolvedTools::from_stated(Some(&["read".to_owned()])).unwrap();
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            mcp: Some(&lane),
            workspace: &ws,
            allow_query_data: true,
            tools: &allow,
            hooks: None,
        },
        &[step("use the server", Capabilities::default())],
    );

    let names = names(&built, 0);
    assert!(
        !names.contains(&"mcp__fixture__echo".to_string()),
        "the list removes the server's tool: {names:?}"
    );
    let refused = built[0]
        .executor
        .execute("mcp__fixture__echo", serde_json::json!({}))
        .await
        .expect_err("the removed name is unknown at the step's door");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );
}
