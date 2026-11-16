//! Child-loop cases wired exactly as the factory composes a child.
//!
//! These scripted-provider cases prove deterministic policy wiring only; they
//! are not evidence about model behavior.

use saya_agent::{
    AgentRunControl, AnsweringLimits, CancellationToken, ToolCall, ToolDefinition,
    run_agent_with_control,
};
use saya_harness::coding::policy::ChildAuthorityView;

use super::child_toolset;
use super::tests::{Sandbox, checkout, implementer_view};
use super::tests_loop::{NoopSink, ScriptedProvider, limits, request};

/// One child composed exactly as the factory composes a workspace-only
/// child: the same toolset builder over the same view, plus the same wrapped
/// approval the factory installs, so the loop below sees the decider a
/// scheduled child sees. `ChildComponents` fields are harness-private, so the
/// wiring is repeated here instead of extracted; the coding lifecycle test
/// covers the factory's own composition end to end.
fn factory_wiring(
    sandbox: &Sandbox,
) -> (
    Vec<ToolDefinition>,
    super::ChildTools,
    super::decider::ExactWriteChildApproval,
    ChildAuthorityView,
) {
    let checkout = checkout("child-loop");
    let view = implementer_view(&checkout);
    let (definitions, tools) = child_toolset(&view, &sandbox.pair(&checkout)).unwrap();
    let approval = super::decider::ExactWriteChildApproval::new(&view, checkout);
    (definitions, tools, approval, view)
}

async fn run_factory_write(sandbox: &Sandbox, path: &str) -> saya_agent::AgentOutput {
    let (definitions, tools, approval, view) = factory_wiring(sandbox);
    let control = AgentRunControl::new(
        limits(&view),
        AnsweringLimits {
            max_requests: Some(2),
            max_tool_calls: Some(2),
            max_reported_tokens: Some(1024),
        },
        &approval,
        &NoopSink,
        CancellationToken::new(),
    );
    let provider = ScriptedProvider {
        call: ToolCall {
            id: "write-1".into(),
            name: "write".into(),
            arguments: serde_json::json!({"path": path, "content": "child bytes"}),
        },
        turn: std::sync::Mutex::new(0),
    };
    run_agent_with_control(&provider, &tools, request(), definitions, control)
        .await
        .unwrap()
}

// Protects the factory-installed approval: a write the view does not hold
// must be denied before execution — no permit is admitted, so no tool-call
// effect settles uncertain and the joined schedule stays settled — while the
// executor keeps its exact admission as defense in depth. The coding
// lifecycle test proves the joined job itself stays complete.
#[tokio::test]
async fn factory_approval_denies_a_write_the_view_does_not_admit() {
    let sandbox = Sandbox::new("factory-outside-path");
    let output = run_factory_write(&sandbox, "../escape.txt").await;

    assert!(!sandbox.outer.join("escape.txt").exists());
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolDenied { .. }))
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}

// Protects the factory-installed approval from over-narrowing: the same
// decider must let any write in the shared working checkout reach the
// existing workspace mutation.
#[tokio::test]
async fn factory_approval_runs_a_write_anywhere_in_the_workspace() {
    let sandbox = Sandbox::new("factory-inside-path");
    let output = run_factory_write(&sandbox, "a/anywhere.txt").await;

    assert!(sandbox.ws_root().join("a/anywhere.txt").is_file());
    assert_eq!(output.tool_metadata[0].status, "completed");
}
