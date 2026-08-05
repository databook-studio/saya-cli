//! Runs an agent prompt on a background thread and streams its events back to
//! the UI over a channel, so the event loop stays responsive (spinner + cancel)
//! while the model works.

use crate::agent::runtime::{PromptOverrides, run_prompt_with_sink};
use crate::config::runtime::RuntimeConfig;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentOutput, ApprovalDecider, ApprovalPolicy, CancellationToken,
    ChatMessage, ToolDefinition,
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

/// Approval decider that honors the session's approval policy: `ReadOnly`
/// auto-approves, `Never` auto-denies, and `Ask` prompts the UI (via the same
/// channel) and waits for the user's y/n answer.
struct ChannelApproval {
    tx: UnboundedSender<StreamMsg>,
    policy: ApprovalPolicy,
}

#[async_trait]
impl ApprovalDecider for ChannelApproval {
    async fn approve(&self, tool: &ToolDefinition) -> bool {
        match self.policy {
            ApprovalPolicy::ReadOnly => return true,
            ApprovalPolicy::Never => return false,
            ApprovalPolicy::Ask => {}
        }
        let (respond, answer) = oneshot::channel();
        if self
            .tx
            .send(StreamMsg::ApprovalRequest {
                tool: tool.name.clone(),
                respond,
            })
            .is_err()
        {
            return false;
        }
        answer.await.unwrap_or(false)
    }
}

/// Spawns the agent on a background thread and returns the live stream handle.
pub(crate) fn start(
    runtime: Arc<RuntimeConfig>,
    prompt: String,
    approval: ApprovalPolicy,
    overrides: PromptOverrides,
    history: Vec<ChatMessage>,
    state_db: SqliteStateStore,
) -> Stream {
    let (tx, rx) = unbounded_channel();
    let cancel = CancellationToken::new();
    let cancel_worker = cancel.clone();
    let prompt_worker = prompt.clone();

    std::thread::spawn(move || {
        let sink = ChannelSink { tx: tx.clone() };
        let decider: Arc<dyn ApprovalDecider> = Arc::new(ChannelApproval {
            tx: tx.clone(),
            policy: approval,
        });
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
        ));
        let _ = tx.send(StreamMsg::Done(result.map_err(|error| error.to_string())));
    });

    Stream { rx, cancel, prompt }
}
