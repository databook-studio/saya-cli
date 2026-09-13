//! Runs an agent prompt on a background thread and streams its events back to
//! the UI over a channel, so the event loop stays responsive (spinner + cancel)
//! while the model works.

use crate::agent::runtime::{PromptOverrides, run_prompt_with_sink};
use crate::config::runtime::RuntimeConfig;
use crate::grant_token::grant_token;
use crate::interactive::session_universe::SessionUniverse;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentOutput, ApprovalChoice, ApprovalDecider, ApprovalDecision,
    ApprovalPolicy, CancellationToken, ChatMessage, SessionPolicy, ToolDefinition,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

/// A message from the agent thread to the UI.
pub(crate) enum StreamMsg {
    Event(AgentEvent),
    /// The agent is asking the user to approve a tool; the UI replies via
    /// `respond` with the user's [`ApprovalChoice`]. `grant` is the grammar
    /// token a session grant for this call would record — the modal offers
    /// its third answer only when it is `Some`.
    ApprovalRequest {
        tool: String,
        /// Human-readable detail (e.g. the SQL) shown so the user sees what they approve.
        detail: Option<String>,
        grant: Option<String>,
        respond: oneshot::Sender<ApprovalChoice>,
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
/// the modal (over the same channel) and the user's [`ApprovalChoice`] is
/// recorded into the session policy and turned back into the decision. The
/// TUI can always prompt, so an ask never falls back to stdin. The policy is
/// the session's one instance, cloned per turn by `start` — a grant recorded
/// through one turn's ask is in force for every later turn.
pub(crate) struct ChannelApproval {
    tx: UnboundedSender<StreamMsg>,
    policy: SessionPolicy,
}

impl ChannelApproval {
    pub(crate) fn new(tx: UnboundedSender<StreamMsg>, policy: SessionPolicy) -> Self {
        Self { tx, policy }
    }
}

#[async_trait]
impl ApprovalDecider for ChannelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        let grant = grant_token(&tool.name, arguments);
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
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
                        grant,
                        respond,
                    })
                    .is_err()
                {
                    return false;
                }
                match answer.await {
                    Ok(choice) => {
                        // The user's answer reaches the session's grant store
                        // here, where the policy lives; allow-once and deny
                        // record nothing.
                        self.policy.record(choice.clone());
                        !matches!(choice, ApprovalChoice::Deny)
                    }
                    // A UI that died mid-ask denies.
                    Err(_) => false,
                }
            }
        }
    }
}

/// Everything one streaming turn runs with. A bundle rather than eight
/// positional parameters, so the call sites read by name. `policy` is the
/// session's one approval policy, cloned into this turn's decider so the
/// session's grant set is shared across turns; `approval` is the same
/// policy's mode, for the turn's definition advertising.
pub(crate) struct StreamRequest {
    pub(crate) runtime: Arc<RuntimeConfig>,
    pub(crate) prompt: String,
    pub(crate) approval: ApprovalPolicy,
    pub(crate) policy: SessionPolicy,
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
        policy,
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
        let decider: Arc<dyn ApprovalDecider> = Arc::new(ChannelApproval::new(tx.clone(), policy));
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
