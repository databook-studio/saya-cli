//! Runs an agent prompt on a background thread and streams its events back to
//! the UI over a channel, so the event loop stays responsive (spinner + cancel)
//! while the model works.

use crate::agent::runtime::{PromptOverrides, run_prompt_with_sink};
use crate::approval_facts::ApprovalFacts;
use crate::config::runtime::RuntimeConfig;
use crate::grant_token::{TurnPrimary, grant_token};
use crate::interactive::session_universe::SessionUniverse;
use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentMode, AgentOutput, ApprovalChoice, ApprovalDecider,
    ApprovalDecision, ApprovalPolicy, CancellationToken, ChatMessage, SessionPolicy,
    ToolDefinition,
};
use saya_store::SqliteStateStore;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;

/// A message from the agent thread to the UI.
pub(crate) enum StreamMsg {
    Event(AgentEvent),
    /// The agent is asking the user to approve a tool; the UI replies via
    /// `respond` with the user's [`ApprovalChoice`]. `detail` is the shared
    /// fact body the terminal prompt renders too (`approval_facts::call_facts`),
    /// so both surfaces state the same facts; `grant` is the grammar
    /// token a session grant for this call would record — the modal offers
    /// its third answer only when it is `Some`.
    ApprovalRequest {
        tool: String,
        /// The per-call fact body (e.g. the SQL, the bounds, the session's
        /// grant history) shown so the user sees what they approve.
        detail: Option<String>,
        grant: Option<String>,
        respond: oneshot::Sender<ApprovalChoice>,
    },
    /// A system fact the decider must say into the transcript — today, that
    /// the session journal could not record a grant the user just made. The
    /// consent stands; the line is missing, and silence would hide it.
    Notice(String),
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
    /// The turn's primary connection, bound by the turn this decider
    /// belongs to; the SQL family's suggestion names it when the call
    /// names no connection.
    primary: TurnPrimary,
    /// The session composition's prompt facts: what this decider's modal
    /// may state. The same bundle the terminal decider renders, so the
    /// modal cannot state different facts.
    facts: ApprovalFacts,
    /// The session journal, when this decider belongs to a session: a
    /// `[s]` answer's new grant is journalled there, before the call it
    /// allowed runs. `None` — test shapes — records grants with no journal.
    journal: Option<Arc<saya_store::SessionJournal>>,
}

impl ChannelApproval {
    pub(crate) fn new(
        tx: UnboundedSender<StreamMsg>,
        policy: SessionPolicy,
        primary: TurnPrimary,
        facts: ApprovalFacts,
        journal: Option<Arc<saya_store::SessionJournal>>,
    ) -> Self {
        Self {
            tx,
            policy,
            primary,
            facts,
            journal,
        }
    }
}

#[async_trait]
impl ApprovalDecider for ChannelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &serde_json::Value) -> bool {
        // Deny first, at every program-named door: a denied name refuses
        // (`false`) before grant lookup, before the modal, before bypass —
        // the loop then relays this decider's typed refusal (see
        // `refusal_detail` below), and the executor's own deny check stays
        // as the second door for any caller that executes without approving.
        if crate::interactive::session_deny::denied_call_program(
            &tool.name,
            arguments,
            &self.facts.denied_programs,
        )
        .is_some()
        {
            return false;
        }
        let primary = self.primary.get();
        let grant = grant_token(&tool.name, arguments, primary.as_deref(), &self.facts);
        match self.policy.resolve(&tool.effect, grant.as_deref()) {
            ApprovalDecision::Allow => true,
            ApprovalDecision::Deny { .. } => false,
            ApprovalDecision::Ask => {
                let (respond, answer) = oneshot::channel();
                // The detail is the shared fact body — the same bytes the
                // terminal prompt renders (`approval_facts::call_facts`) —
                // so the modal cannot state different facts for the same
                // call. `None` and the modal falls back to the tool's name.
                let detail = crate::approval_facts::call_facts(
                    &tool.name,
                    arguments,
                    grant.as_deref(),
                    &self.facts,
                    primary.as_deref(),
                    Some(self.policy.grants()),
                );
                if self
                    .tx
                    .send(StreamMsg::ApprovalRequest {
                        tool: tool.name.clone(),
                        detail,
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
                        // record nothing. A *new* grant is journalled right
                        // here — before this call is allowed to run — by the
                        // shared operation, one wording. A failed journal
                        // write changes no consent: it is said into the
                        // transcript, never silent.
                        let (_, warning) = crate::interactive::session_grants::record_prompt_answer(
                            &self.policy,
                            &choice,
                            self.journal.as_deref(),
                        );
                        if let Some(warning) = warning {
                            let _ = self.tx.send(StreamMsg::Notice(warning));
                        }
                        !matches!(choice, ApprovalChoice::Deny)
                    }
                    // A UI that died mid-ask denies.
                    Err(_) => false,
                }
            }
        }
    }

    fn refusal_detail(
        &self,
        tool: &ToolDefinition,
        arguments: &serde_json::Value,
    ) -> Option<String> {
        // The loop reads this only after `approve` denied, so the same pure
        // seam answers the wording: the typed refusal for a denied program,
        // `None` for every denial this decider did not word (mode denials,
        // the modal's deny, a dead UI), where the loop's generic denial stands.
        crate::interactive::session_deny::denied_call_program(
            &tool.name,
            arguments,
            &self.facts.denied_programs,
        )
        .map(|program| crate::interactive::session_deny::denied_refusal(&program))
    }
}

/// Everything one streaming turn runs with. A bundle rather than eight
/// positional parameters, so the call sites read by name. `policy` is the
/// session's one approval policy, cloned into this turn's decider so the
/// session's grant set is shared across turns; `approval` is the same
/// policy's mode, for the turn's definition advertising. `journal` is the
/// session's journal, when the turn belongs to a session.
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
    pub(crate) journal: Option<Arc<saya_store::SessionJournal>>,
    // The agent's task posture, threaded like `approval`: the real source
    // arrives with `/mode` in the next slice.
    pub(crate) agent_mode: AgentMode,
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
        journal,
        agent_mode,
    } = request;
    let (tx, rx) = unbounded_channel();
    let cancel = CancellationToken::new();
    let cancel_worker = cancel.clone();
    let prompt_worker = prompt.clone();
    // The turn's primary handle rides the session's universe: the decider
    // holds a clone, and the turn binds the registry's primary into it.
    let primary = session.primary.clone();
    // The prompt facts come from the members this session actually composed,
    // read off the universe and the resolved config — the modal states only
    // these.
    let facts = session.approval_facts(&runtime);

    std::thread::spawn(move || {
        let sink = ChannelSink { tx: tx.clone() };
        let decider: Arc<dyn ApprovalDecider> = Arc::new(ChannelApproval::new(
            tx.clone(),
            policy,
            primary,
            facts,
            journal,
        ));
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
            agent_mode,
        ));
        let _ = tx.send(StreamMsg::Done(result.map_err(|error| error.to_string())));
    });

    Stream { rx, cancel, prompt }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
