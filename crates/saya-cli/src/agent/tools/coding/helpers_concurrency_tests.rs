//! Several `helpers` calls in one turn run at the same time. The `helpers`
//! description promises it and the tool is `ToolConcurrency::Concurrent`, and
//! the runner overlaps a batch only when every member is also allowed without
//! a per-call approval — so a `helpers` definition that required one would run
//! the turn's helpers serially. Two `helpers` calls whose executions block
//! until both have started, with a timeout so a serial run fails instead of
//! hanging.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, ApprovalDecider, CancellationToken, ChatProvider, ChatRequest,
    ChatResponse, NoopEventSink, ProviderError, ProviderEvent, ProviderStream, ToolCall,
    ToolDefinition, ToolError, ToolExecutor, run_agent_with_sink,
};
use serde_json::json;
use tokio::sync::{Notify, Semaphore};

use super::task_tool;

/// Every `helpers` call blocks until the other has started, so the turn can
/// finish only if the runner ran the two calls at the same time. A serial
/// runner leaves the first call waiting on the latch, and the test's own
/// timeout fails it instead of hanging.
struct LatchHelpers {
    started: AtomicUsize,
    both_started: Notify,
    release: Semaphore,
}

impl LatchHelpers {
    fn new() -> Self {
        Self {
            started: AtomicUsize::new(0),
            both_started: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}

#[async_trait]
impl ToolExecutor for LatchHelpers {
    async fn execute(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if self.started.fetch_add(1, Ordering::AcqRel) + 1 == 2 {
            self.both_started.notify_one();
        }
        self.release
            .acquire()
            .await
            .expect("the test gate stays open")
            .forget();
        Ok(json!({"summary": "helper finished"}))
    }
}

/// Approves every call. The point of the test is overlap, not approval: a real
/// coding root auto-approves its advertised `helpers` definition, so this
/// stands in for that decider and nothing else.
struct ApproveHelpers;

#[async_trait]
impl ApprovalDecider for ApproveHelpers {
    async fn approve(&self, _: &ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

struct TwoHelpersProvider {
    turn: std::sync::Mutex<u8>,
}

#[async_trait]
impl ChatProvider for TwoHelpersProvider {
    fn name(&self) -> &str {
        "helpers-concurrency"
    }

    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("the streaming provider path is used")
    }

    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut turn = self.turn.lock().expect("turn counter is available");
            let first = *turn == 0;
            *turn += 1;
            first
        };
        let events = if first {
            // Both helpers calls arrive in ONE turn: that is what makes them
            // overlap (a call in its own turn runs alone whatever its tool is).
            vec![
                Ok(ProviderEvent::ToolCalls(vec![
                    helper_call("call-helper-a", "child-a", "write new-a.txt"),
                    helper_call("call-helper-b", "child-b", "write new-b.txt"),
                ])),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("both helpers ran".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

fn helper_call(id: &str, agent: &str, description: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "helpers".into(),
        arguments: json!({
            "description": description,
            "prompt": format!("write for {agent}"),
            "subagent_type": "general",
            "agent_id": agent,
        }),
    }
}

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "hand the two pieces to two helpers".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

#[tokio::test]
async fn two_helpers_calls_in_one_turn_run_at_the_same_time() {
    let tools = Arc::new(LatchHelpers::new());
    let provider = TwoHelpersProvider {
        turn: std::sync::Mutex::new(0),
    };
    // The coding root permits workspace writes and `helpers` keeps its
    // WriteWorkspace local state, so the turn runs under the same permit the
    // root loop uses; otherwise the workspace gate would deny the calls before
    // the concurrency question arises.
    let limits = AgentLimits {
        permit_workspace_writes: true,
        ..AgentLimits::default()
    };
    let run = {
        let tools = Arc::clone(&tools);
        tokio::spawn(async move {
            run_agent_with_sink(
                &provider,
                tools.as_ref(),
                request(),
                vec![task_tool::definition(&saya_config::HelperAgents::default())],
                limits,
                &ApproveHelpers,
                &NoopEventSink,
                CancellationToken::new(),
            )
            .await
        })
    };
    let overlapped = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if tools.started.load(Ordering::Acquire) == 2 {
                break;
            }
            tools.both_started.notified().await;
        }
    })
    .await
    .is_ok();

    // Release both results on success and feature-red timeout before awaiting
    // the loop, so the assertion cannot strand either test future.
    tools.release.add_permits(2);
    let output = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("the released tool loop must finish")
        .expect("the tool loop task must join")
        .expect("two admitted helpers calls and the final answer must succeed");

    assert!(
        overlapped,
        "both helpers calls must be in flight before either returns"
    );
    assert_eq!(
        output.tool_metadata.len(),
        2,
        "the turn ran both helpers calls"
    );
    assert!(
        output
            .tool_metadata
            .iter()
            .all(|metadata| metadata.name == "helpers"),
        "both calls ran the helpers tool"
    );
}
