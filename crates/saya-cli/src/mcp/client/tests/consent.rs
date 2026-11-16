//! How a call to an MCP tool is consented.
//!
//! A server is a program whose effects saya cannot bound, so the rule is the
//! `shell`'s own: `bypass` runs it, `ask` puts each call to the operator, and a
//! headless `ask` refuses with the flag that would reach it named. These tests
//! pin the decision, the reason a denial gives, and the line the operator sees.

use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolDefinition, ToolEffect, ToolExecutor};
use saya_agent::{LocalStateEffect, ToolConcurrency};
use serde_json::json;

use super::super::support::{Fixture, TIMEOUTS, lane, recording_operator};

/// The definition of `name`, as this client advertises it.
fn definition(lane: &super::super::McpTools, name: &str) -> ToolDefinition {
    lane.definitions()
        .into_iter()
        .find(|candidate| candidate.name == name)
        .unwrap_or_else(|| panic!("{name} is advertised; reports: {:?}", lane.reports()))
}

/// A definition no server owns, for the routing tests.
fn foreign() -> ToolDefinition {
    ToolDefinition {
        name: "shell".to_owned(),
        description: String::new(),
        read_only: false,
        parameters: json!({}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

/// `ask` puts the call to the operator, whose line names the caller, the
/// server, and the tool — and an approval runs it.
#[tokio::test]
async fn ask_puts_the_call_to_the_operator() {
    let fixture = Fixture::new("consent-ask");
    let servers = fixture.servers(&[], TIMEOUTS);
    let operator = recording_operator(true);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Ask,
        Some(operator.clone()),
    )
    .expect("a lane is composed");
    let echo = definition(&lane, "mcp__fixture__echo");
    assert!(lane.approve(&echo, &json!({ "text": "hi" })).await);
    assert_eq!(
        operator.subjects(),
        ["Call an MCP tool (server \"fixture\", tool \"echo\")"]
    );
    assert!(lane.execute("mcp__fixture__echo", json!({})).await.is_ok());
}

/// A denial is a denial: the operator said no, so the call does not run and no
/// reason beyond the operator's own answer is invented for it.
#[tokio::test]
async fn a_denied_ask_does_not_run() {
    let fixture = Fixture::new("consent-denied");
    let servers = fixture.servers(&[], TIMEOUTS);
    let operator = recording_operator(false);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Ask,
        Some(operator.clone()),
    )
    .expect("a lane is composed");
    let echo = definition(&lane, "mcp__fixture__echo");
    assert!(!lane.approve(&echo, &json!({})).await);
    assert_eq!(operator.subjects().len(), 1);
    assert!(lane.refusal_detail(&echo, &json!({})).is_none());
}

/// A headless `ask` has nobody to put the call to, so it refuses and names the
/// flag that reaches a server from such a driver. The reason is the same one
/// the call itself carries, so the model reads it whichever path it took.
#[tokio::test]
async fn a_headless_ask_refuses_and_names_the_flag() {
    let fixture = Fixture::new("consent-headless");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(&servers, &fixture.resolver(&[]), ApprovalPolicy::Ask, None)
        .expect("a lane is composed");
    let echo = definition(&lane, "mcp__fixture__echo");
    assert!(!lane.approve(&echo, &json!({})).await);
    let detail = lane
        .refusal_detail(&echo, &json!({}))
        .expect("a headless refusal states why");
    assert!(detail.contains("--approval-mode bypass"), "{detail}");
    let error = lane
        .execute("mcp__fixture__echo", json!({}))
        .await
        .expect_err("a headless ask refuses the call");
    assert!(
        error.to_string().contains("--approval-mode bypass"),
        "{error}"
    );
}

/// `bypass` runs the call without asking: the mode is the operator's standing
/// answer, and nothing is put to anyone.
#[tokio::test]
async fn bypass_runs_without_asking() {
    let fixture = Fixture::new("consent-bypass");
    let servers = fixture.servers(&[], TIMEOUTS);
    let operator = recording_operator(false);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        Some(operator.clone()),
    )
    .expect("a lane is composed");
    let echo = definition(&lane, "mcp__fixture__echo");
    assert!(lane.approve(&echo, &json!({})).await);
    assert!(lane.execute("mcp__fixture__echo", json!({})).await.is_ok());
    assert!(operator.subjects().is_empty(), "bypass asks nobody");
}

/// The two modes that reach no server state why in their own words: a
/// read-only mode cannot bound a program, and a mode that calls nothing says
/// so. The text is what a surface reports for a call it has no client to route.
#[test]
fn the_no_server_modes_state_their_own_reason() {
    let fixture = Fixture::new("consent-modes");
    let servers = fixture.servers(&[], TIMEOUTS);
    for policy in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        assert!(
            lane(&servers, &fixture.resolver(&[]), policy, None).is_none(),
            "{policy:?} composes no lane"
        );
    }
    assert!(super::super::tools::READ_ONLY_REFUSAL.contains("read-only"));
    assert!(super::super::tools::NEVER_REFUSAL.contains("calls no MCP tool"));
}

/// A name this client does not own is not routed here at all — which is what
/// lets a surface hand its own tools to their own deciders first.
#[tokio::test]
async fn a_foreign_name_is_not_owned() {
    let fixture = Fixture::new("consent-routing");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let shell = foreign();
    assert!(!lane.approve(&shell, &json!({})).await);
    assert!(lane.refusal_detail(&shell, &json!({})).is_some());
}
