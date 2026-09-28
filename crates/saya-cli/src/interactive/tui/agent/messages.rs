//! The UI-bound message channel for one streaming turn.

use crate::agent::tools::{AgentCapture, CaptureRefusalReason};
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
    /// The typed result of one successful agent `bounded_sql_query` — exactly
    /// what the model saw, sent before the loop's `ToolCompleted` for the
    /// same call. Consumed by the capture pairing (C2).
    QueryCaptured(AgentCapture),
    /// One successful `bounded_sql_query` was refused — the model's view was
    /// truncated or redacted, or the result is over the accounted capture
    /// budget: nothing is held, not even a partial. `reason` is what the
    /// snapshot's refusal names (R3). Consumed by the capture pairing (C2).
    QueryCaptureRefused {
        sql: String,
        connection: String,
        reason: CaptureRefusalReason,
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
