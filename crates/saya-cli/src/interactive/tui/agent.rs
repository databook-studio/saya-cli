//! Runs an agent prompt on a background thread and streams its events back to
//! the UI over a channel, so the event loop stays responsive (spinner + cancel)
//! while the model works.

use crate::agent::runtime::{PromptOverrides, run_prompt_with_sink};
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_universe::SessionUniverse;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentOutput, ApprovalDecider, ApprovalDecision, ApprovalPolicy,
    CancellationToken, ChatMessage, SessionPolicy, ToolDefinition,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

/// A message from the agent thread to the UI.
pub(crate) enum StreamMsg {
    Event(AgentEvent),
    /// The agent is asking the user to approve a tool; the UI replies via `respond`.
    ApprovalRequest {
        tool: String,
        /// Human-readable detail (e.g. the SQL) shown so the user sees what they approve.
        detail: Option<String>,
        respond: oneshot::Sender<bool>,
    },
    Done(Result<AgentOutput, String>),
}

/// A running agent request the UI drains each tick.
pub(crate) struct Stream {
    pub(crate) rx: UnboundedReceiver<StreamMsg>,
    pub(crate) cancel: CancellationToken,
    pub(crate) prompt: String,
}

/// Sink that forwards every agent event to the UI channel.
struct ChannelSink {
    tx: UnboundedSender<StreamMsg>,
}

#[async_trait]
impl AgentEventSink for ChannelSink {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.tx.send(StreamMsg::Event(event));
    }
}

/// Approval decider that consults the session policy: whatever the engine
/// allows or denies runs or refuses without the user; an ask is rendered as
/// the modal (over the same channel) and the user's y/n answer is the
/// decision. The TUI can always prompt, so an ask never falls back to stdin.
pub(crate) struct ChannelApproval {
    tx: UnboundedSender<StreamMsg>,
    policy: SessionPolicy,
}

impl ChannelApproval {
    pub(crate) fn new(tx: UnboundedSender<StreamMsg>, policy: ApprovalPolicy) -> Self {
        Self {
            tx,
            policy: SessionPolicy::new(policy),
        }
    }
}

#[async_trait]
impl ApprovalDecider for ChannelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        match self.policy.resolve(&tool.effect, None) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny => false,
            ApprovalDecision::Ask => {
                let (respond, answer) = oneshot::channel();
                if self
                    .tx
                    .send(StreamMsg::ApprovalRequest {
                        tool: tool.name.clone(),
                        detail: crate::agent::tools::sql_tool_call(&tool.name, arguments).map(
                            |call| match call.target {
                                Some(target) => format!("-- on {target}\n{}", call.sql),
                                None => call.sql,
                            },
                        ),
                        respond,
                    })
                    .is_err()
                {
                    return false;
                }
                answer.await.unwrap_or(false)
            }
        }
    }
}

/// Everything one streaming turn runs with. A bundle rather than eight
/// positional parameters, so the call sites read by name.
pub(crate) struct StreamRequest {
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) prompt: String,
    pub(crate) approval: ApprovalPolicy,
    pub(crate) overrides: PromptOverrides,
    pub(crate) history: Vec<ChatMessage>,
    pub(crate) state_db: SqliteStateStore,
    pub(crate) last_sql: Option<String>,
    pub(crate) session: Arc<SessionUniverse>,
}

/// Spawns the agent on a background thread and returns the live stream handle.
pub(crate) fn start(request: StreamRequest) -> Stream {
    let StreamRequest {
        runtime,
        prompt,
        approval,
        overrides,
        history,
        state_db,
        last_sql,
        session,
    } = request;
    let (tx, rx) = unbounded_channel();
    let cancel = CancellationToken::new();
    let cancel_worker = cancel.clone();
    let prompt_worker = prompt.clone();

    std::thread::spawn(move || {
        let sink = ChannelSink { tx: tx.clone() };
        let decider: Arc<dyn ApprovalDecider> =
            Arc::new(ChannelApproval::new(tx.clone(), approval));
        let runtime_handle = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = tx.send(StreamMsg::Done(Err(error.to_string())));
                return;
            }
        };
        let result = runtime_handle.block_on(run_prompt_with_sink(
            runtime.as_ref(),
            &prompt_worker,
            approval,
            false, // never prompt on stdin: the TUI collects approvals via a modal
            overrides,
            history,
            &sink,
            cancel_worker,
            Some(state_db),
            Some(decider),
            last_sql,
            Some(session),
        ));
        let _ = tx.send(StreamMsg::Done(result.map_err(|error| error.to_string())));
    });

    Stream { rx, cancel, prompt }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
