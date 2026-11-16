//! `Stop` on the helper surface: a helper's agent loop ends inside the run
//! engine, so the same hook runner the root uses must fire when it does.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, AllowReadOnlyApproval, AnsweringLimits, CancellationToken,
    ChatMessage, ChatProvider, ChatRequest, ChatResponse, ProviderError, ToolError, ToolExecutor,
};
use saya_harness::coding::child::{
    ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError,
};
use saya_harness::coding::coordinator::{
    ChildEvent, ChildEventReceiver, child_event_channel, run_child,
};
use serde_json::Value;

use super::tests::{Sandbox, checkout, implementer_view};
use crate::agent::hooks::test_support::stop_hooks;
use crate::agent::hooks::{HookCaller, HookGate, Hooks};

struct AnsweringProvider;

#[async_trait]
impl ChatProvider for AnsweringProvider {
    fn name(&self) -> &str {
        "stop-script"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse::new(ChatMessage::text("assistant", "done")))
    }
}

struct NoTools;

#[async_trait]
impl ToolExecutor for NoTools {
    async fn execute(&self, _: &str, _: Value) -> Result<Value, ToolError> {
        Ok(serde_json::json!({}))
    }
}

/// A child factory whose executor is the real hook gate, so the harness's own
/// loop-end call is what runs the configured `Stop` hook.
struct StopFactory {
    hooks: Arc<Hooks>,
}

impl ChildFactory for StopFactory {
    fn build(
        &self,
        _spec: &ChildExecutionSpec,
        _cancellation: &CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        Ok(ChildComponents::new(
            Box::new(AnsweringProvider),
            Box::new(HookGate::new(Arc::clone(&self.hooks), Arc::new(NoTools))),
            Vec::new(),
            Box::new(AllowReadOnlyApproval),
        ))
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "finish the assignment".to_owned(),
        profile_names: Vec::new(),
        model: "test-model".to_owned(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

fn limits() -> (AgentLimits, AnsweringLimits) {
    (
        AgentLimits {
            max_turns: Some(2),
            max_tool_calls: Some(4),
            max_continuations: Some(0),
            permit_candidate_writes: false,
            permit_workspace_writes: false,
            permit_external_effects: false,
            context_byte_budget: 64 * 1024,
            progress: None,
        },
        AnsweringLimits {
            max_requests: Some(2),
            max_tool_calls: Some(4),
            max_reported_tokens: Some(1_000),
        },
    )
}

async fn drain(mut receiver: ChildEventReceiver) -> Vec<ChildEvent> {
    let mut events = Vec::new();
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    events
}

/// A helper's loop end runs its `Stop` hook exactly once, with the payload
/// shape the root's `Stop` gets: the event, the job, and the helper's own
/// agent id.
#[tokio::test]
async fn a_stop_hook_runs_once_when_a_helpers_loop_ends() {
    let sandbox = Sandbox::new("stop-helper");
    let checkout = checkout("stop-helper");
    let seen = sandbox.outer.join("stop.json");
    let hooks = stop_hooks(
        &format!("cat >> '{}'", seen.display()),
        &sandbox.ws_root(),
        HookCaller::Helper {
            agent_id: "agent".to_owned(),
            job_id: "job".to_owned(),
        },
    );
    let factory = StopFactory { hooks };
    let view = implementer_view(&checkout);
    let (agent_limits, answering) = limits();
    let spec = ChildExecutionSpec::new(view, request(), agent_limits, answering, None)
        .expect("the helper spec is valid");
    let (sender, receiver) = child_event_channel(&spec);

    let (output, _events) = tokio::join!(
        run_child(spec, &factory, sender, CancellationToken::new()),
        drain(receiver)
    );
    output.expect("the helper's loop must finish");

    let recorded = std::fs::read_to_string(&seen).expect("the stop hook must have run");
    assert_eq!(
        recorded.matches("\"hook_event_name\":\"Stop\"").count(),
        1,
        "Stop must run exactly once when the helper's loop ends: {recorded}"
    );
    assert!(
        recorded.contains("\"job_id\":\"job\"") && recorded.contains("\"agent_id\":\"agent\""),
        "the payload must name the job and the helper: {recorded}"
    );
}
