//! Child workspace calls at the real agent-loop boundary.
//!
//! These scripted-provider cases prove deterministic policy wiring only; they
//! are not evidence about model behavior.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream;
use saya_agent::{
    AgentEventSink, AgentLimits, AgentRequest, AgentRunControl, AnsweringLimits, ApprovalDecider,
    CancellationToken, ChatProvider, ChatRequest, ChatResponse, ProviderError, ProviderEvent,
    ProviderStream, ToolCall, ToolDefinition, run_agent_with_control,
};
use saya_harness::coding::policy::{ChildApprovalDecider, ChildAuthorityView};

#[path = "tests_loop_settlement.rs"]
mod settlement;

use super::child_toolset;
use super::compose::project_agent_limits;
use super::tests::{Sandbox, checkout, implementer_view, read_only_view};

struct ScriptedProvider {
    call: ToolCall,
    turn: std::sync::Mutex<u8>,
}

#[async_trait]
impl ChatProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "child-loop-script"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the agent loop uses the stream path")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let call = {
            let mut turn = self.turn.lock().unwrap();
            let call = (*turn == 0).then(|| self.call.clone());
            *turn = (*turn).saturating_add(1);
            call
        };
        let events = match call {
            Some(call) => vec![
                Ok(ProviderEvent::ToolCalls(vec![call])),
                Ok(ProviderEvent::Done),
            ],
            None => vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Done),
            ],
        };
        Ok(Box::pin(stream::iter(events)))
    }
}

struct NoopSink;

#[async_trait]
impl AgentEventSink for NoopSink {
    async fn emit(&self, _: saya_agent::AgentEvent) {}
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "write the assigned child file".into(),
        profile_names: Vec::new(),
        model: "scripted".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn limits(view: &saya_harness::coding::policy::ChildAuthorityView) -> AgentLimits {
    project_agent_limits(
        view,
        false,
        AgentLimits {
            max_turns: Some(3),
            max_tool_calls: Some(4),
            max_continuations: Some(0),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 16 * 1024,
            progress: None,
        },
    )
}

// Protects the broad loop permit's projection: the write capability, a
// command scope, or a shell must open the loop gate; a view with none must
// keep it closed. Existing harness tests do not cover this CLI-owned
// AgentLimits projection.
#[test]
fn projected_limits_follow_narrowed_authority() {
    let checkout = checkout("loop-limits");
    let write_view = implementer_view(&checkout);
    assert!(write_view.workspace_write());
    assert!(limits(&write_view).permit_workspace_writes);

    let empty_view = read_only_view(&checkout);
    assert!(!limits(&empty_view).permit_workspace_writes);

    // A helper with a shell reaches the shell's own lane decision, so the
    // loop gate must open for it even with no write capability and no command
    // scope — the `explore` shape.
    let shell_limits = project_agent_limits(
        &empty_view,
        true,
        AgentLimits {
            permit_workspace_writes: false,
            ..AgentLimits::default()
        },
    );
    assert!(
        shell_limits.permit_workspace_writes,
        "a helper's shell is a write-shaped tool the loop must let through"
    );

    let projected = project_agent_limits(
        &empty_view,
        false,
        AgentLimits {
            permit_candidate_writes: true,
            permit_external_effects: true,
            ..AgentLimits::default()
        },
    );
    assert!(!projected.permit_candidate_writes);
    assert!(!projected.permit_external_effects);
}

async fn run_write(sandbox: &Sandbox, path: &str, permit: bool) -> saya_agent::AgentOutput {
    let checkout = checkout("child-loop");
    let view = implementer_view(&checkout);
    let (definitions, tools) = child_toolset(&view, &sandbox.pair(&checkout)).unwrap();
    let approval = ChildApprovalDecider::from_view(&view);
    let sink = NoopSink;
    let mut agent_limits = limits(&view);
    agent_limits.permit_workspace_writes &= permit;
    let control = AgentRunControl::new(
        agent_limits,
        AnsweringLimits {
            max_requests: Some(2),
            max_tool_calls: Some(2),
            max_reported_tokens: Some(1024),
        },
        &approval,
        &sink,
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

// Protects the projection plus loop/decider/executor composition: a plausible
// regression drops the projected permit and an exact admitted write never
// reaches the existing workspace mutation. Existing harness view tests and
// direct child executor tests do not cover this agent-loop boundary.
#[tokio::test]
async fn child_loop_runs_a_write_inside_the_scoped_path() {
    let sandbox = Sandbox::new("loop-admitted");
    let output = run_write(&sandbox, "a/allowed.txt", true).await;

    assert!(sandbox.ws_root().join("a/allowed.txt").is_file());
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolCompleted { .. }))
    );
    assert_eq!(output.tool_metadata[0].status, "completed");
}

// Protects exact per-call admission through the loop: a broad loop permit and
// the coarse decider cannot turn a path the workspace cannot hold into a
// write. Existing executor tests call the adapter directly and do not prove
// loop delivery.
#[tokio::test]
async fn child_loop_refuses_a_write_outside_the_workspace() {
    let sandbox = Sandbox::new("loop-outside-path");
    let output = run_write(&sandbox, "../escape.txt", true).await;

    assert!(!sandbox.outer.join("escape.txt").exists());
    assert_eq!(output.tool_metadata[0].status, "failed");
    assert!(output.events.iter().any(|event| matches!(
        event,
        saya_agent::AgentEvent::ToolCompleted { name, .. } if name == "write"
    )));
}

// Protects the independent structural loop gate: even when the same narrowed
// view and child decider admit this exact path, a stricter run limit refuses
// before the executor can write it. No test-only production seam is required.
#[tokio::test]
async fn child_loop_limit_can_refuse_an_admitted_path() {
    let sandbox = Sandbox::new("loop-limit");
    let output = run_write(&sandbox, "a/allowed.txt", false).await;

    assert!(!sandbox.ws_root().join("a/allowed.txt").exists());
    assert!(
        output
            .events
            .iter()
            .any(|event| matches!(event, saya_agent::AgentEvent::ToolDenied { .. }))
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}

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

#[test]
fn exact_child_decider_marks_only_a_refused_write_as_immutable() {
    let sandbox = Sandbox::new("typed-refusal");
    let checkout = checkout("child-loop");
    let pair = sandbox.pair(&checkout);
    let writable = implementer_view(&checkout);
    let write = child_toolset(&writable, &pair)
        .unwrap()
        .0
        .into_iter()
        .find(|definition| definition.name == "write")
        .expect("write is advertised for a write-capable view");
    let call = serde_json::json!({"path":"a/anywhere.txt","content":"ignored"});

    // A write-capable view admits any path in W, so nothing is denied as an
    // immutable authority refusal.
    let approval = super::decider::ExactWriteChildApproval::new(&writable, checkout.clone());
    assert_eq!(approval.immutable_authority_refusal(&write, &call), None);

    // The same call against a view with no write capability is refused before
    // execution and named as the immutable class.
    let read_only = read_only_view(&checkout);
    let approval = super::decider::ExactWriteChildApproval::new(&read_only, checkout);
    assert_eq!(
        approval.immutable_authority_refusal(&write, &call),
        Some(saya_agent::ImmutableAuthorityRefusal::OutsideEffectiveWritePaths)
    );
}

// Protects the loop wiring end to end: a read-only (`explore`) helper
// advertises `shell`, the loop's projected permit lets the write-shaped tool
// reach the child decider, and the shell's own lane runs the call in the
// sandbox — where nothing is writable — instead of the loop refusing it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn an_explore_helpers_sandboxed_shell_runs_through_the_loop() {
    let sandbox = Sandbox::new("loop-explore-shell");
    let checkout = checkout("loop-explore-shell");
    let view = read_only_view(&checkout);
    assert!(!view.workspace_write());
    let (mut definitions, mut tools) = child_toolset(&view, &sandbox.pair(&checkout)).unwrap();
    let Some(shell) =
        super::tests_shell::helper_shell(&sandbox, saya_agent::ApprovalPolicy::ReadOnly, None)
    else {
        return;
    };
    definitions.extend(tools.attach_shell(Arc::clone(&shell)));
    let approval = crate::agent::tools::consent::OwnedToolApproval::new(
        vec![shell as Arc<dyn crate::agent::tools::consent::OwnApproval>],
        Arc::new(ChildApprovalDecider::from_view(&view)),
    );

    let agent_limits = project_agent_limits(
        &view,
        true,
        AgentLimits {
            max_turns: Some(3),
            max_tool_calls: Some(4),
            max_continuations: Some(0),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 16 * 1024,
            progress: None,
        },
    );
    let control = AgentRunControl::new(
        agent_limits,
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
            id: "shell-1".into(),
            name: "shell".into(),
            arguments: serde_json::json!({"command": "true", "sandbox": true}),
        },
        turn: std::sync::Mutex::new(0),
    };
    let output = run_agent_with_control(&provider, &tools, request(), definitions, control)
        .await
        .unwrap();
    assert_eq!(
        output.tool_metadata[0].status, "completed",
        "the sandboxed call reached the shell lane instead of the loop gate: {:?}",
        output.events
    );
}
