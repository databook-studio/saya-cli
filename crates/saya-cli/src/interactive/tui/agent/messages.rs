//! The UI-bound message channel for one streaming turn.

use saya_agent::{AgentEvent, AgentOutput, ApprovalChoice, CancellationToken};
use tokio::sync::mpsc::UnboundedReceiver;
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
