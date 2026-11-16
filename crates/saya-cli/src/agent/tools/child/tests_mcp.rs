//! A helper carries the job's MCP lane: the servers the job's `[mcp_servers]`
//! table names, advertised beside its other tools, called through the same
//! lane the root uses, and narrowed by the job's `[tools] allow` list.
//!
//! The helper's lane is its own connection: the composition root builds it
//! when the helper is composed and drops it when the helper ends, so the
//! lifetime is the surface's.

use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy, CancellationToken, ToolError, ToolExecutor};
use saya_config::{MapSecretResolver, ResolvedTools};
use saya_types::{AgentId, JobId};
use serde_json::json;

use super::CliChildFactory;
use super::tests::{Sandbox, checkout, implementer_view};
use crate::agent::tools::consent::Caller;
use crate::mcp::client::McpTools;
use crate::mcp::client::support::{Fixture, RecordingOperator, TIMEOUTS, recording_operator};

/// The helper's lane, composed exactly as the job layer composes it: the
/// job's servers, its mode, its operator, and the helper's own caller line.
fn helper_lane(fixture: &Fixture) -> (Arc<McpTools>, Arc<RecordingOperator>) {
    let servers = fixture.servers(&[], TIMEOUTS);
    let resolver = fixture.resolver(&[]);
    let operator = recording_operator(true);
    let lane = McpTools::connect(
        &servers,
        &resolver,
        ApprovalPolicy::Ask,
        Some(operator.clone() as Arc<dyn crate::agent::tools::consent::Operator>),
        &Caller::Helper {
            agent_id: AgentId::parse("agent").unwrap(),
            job_id: JobId::parse("job").unwrap(),
        },
        CancellationToken::new(),
    )
    .expect("the fixture lane composes");
    (lane, operator)
}

fn factory(sandbox: &Sandbox, checkout_id: &saya_types::ManagedCheckoutId) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        super::tests_factory::test_ai(),
        Arc::new(MapSecretResolver::new([])),
    )
}

/// The helper advertises the server's tool, puts the call to the operator
/// under the helper's own line, and its door reaches the server.
#[tokio::test]
async fn a_helper_advertises_and_reaches_the_servers_tools() {
    let fixture = Fixture::new("helper-mcp");
    let (lane, operator) = helper_lane(&fixture);
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("helper-mcp");
    let factory = factory(&sandbox, &checkout_id).with_mcp(Some(Arc::clone(&lane)));

    let (definitions, tools, _approval) =
        factory.build_toolset(&view).expect("the helper composes");
    let echo = definitions
        .iter()
        .find(|definition| definition.name == "mcp__fixture__echo")
        .unwrap_or_else(|| {
            panic!(
                "the helper advertises the server's tool: {:?}",
                definitions
                    .iter()
                    .map(|definition| definition.name.as_str())
                    .collect::<Vec<_>>()
            )
        })
        .clone();
    assert!(lane.approve(&echo, &json!({ "text": "hi" })).await);
    assert_eq!(
        operator.subjects(),
        ["Helper agent wants to call an MCP tool for job job (server \"fixture\", tool \"echo\")"]
    );

    let result = tools
        .execute("mcp__fixture__echo", json!({ "text": "hi" }))
        .await
        .expect("the helper's door reaches the server");
    assert!(
        result.to_string().contains("echo"),
        "the server's answer reaches the helper: {result}"
    );
}

/// The job's `[tools] allow` list governs the helper's MCP tools exactly as
/// it governs every other name: a list that names neither spelling takes the
/// tool from the advertisement and from the dispatch door.
#[tokio::test]
async fn the_tools_allow_list_removes_the_servers_tools_from_a_helper() {
    let fixture = Fixture::new("helper-mcp-allow");
    let (lane, _operator) = helper_lane(&fixture);
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("helper-mcp-allow");
    let allow = ResolvedTools::from_stated(Some(&["read".to_owned()])).unwrap();
    let factory = factory(&sandbox, &checkout_id)
        .with_mcp(Some(lane))
        .with_tools(allow);

    let (definitions, tools, _approval) =
        factory.build_toolset(&view).expect("the helper composes");
    assert!(
        !definitions
            .iter()
            .any(|definition| definition.name == "mcp__fixture__echo"),
        "the list removes the server's tool: {:?}",
        definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect::<Vec<_>>()
    );
    let refused = tools
        .execute("mcp__fixture__echo", json!({}))
        .await
        .expect_err("the removed name is unknown at the door");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );
}
