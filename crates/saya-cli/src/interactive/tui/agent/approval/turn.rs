//! The turn boundary: what one streaming turn runs with, and what the TUI
//! states about itself there.

use crate::agent::question_scope::QuestionScope;
use crate::agent::runtime::PromptOverrides;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::universe::SessionUniverse;
use saya_agent::{AgentMode, ApprovalPolicy, ChatMessage, SessionPolicy};
use saya_store::SqliteStateStore;
use std::sync::Arc;

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
    // The agent's task posture, threaded like `approval`: the session's
    // `/mode` state at the composition root.
    pub(crate) agent_mode: AgentMode,
    pub(crate) prior_tool_outcomes:
        Option<crate::interactive::session::continuation::PriorToolOutcomes>,
    pub(crate) continuation: Option<saya_store::StoredContinuation>,
    pub(crate) compaction_summary: Option<String>,
    /// The question scope this turn began with: `Some` on the session paths —
    /// the runtime runs on the scope's token and the stream-finalize settles
    /// by its identity. `None` only where no session question was begun.
    pub(crate) question_scope: Option<QuestionScope>,
}

/// What the TUI states about itself at the turn boundary, as the
/// (`can_prompt`, `can_obtain_approval`) pair the turn runners consume.
///
/// `can_prompt` means "this surface may read stdin": always false here —
/// the TUI must never read stdin under the alternate screen. There is a
/// second, explicitly named capability, `can_obtain_approval`: "this surface
/// can obtain a per-call approval at all" — always true here, through the
/// modal. The two flags must never be merged back into one: one boolean
/// meaning two things is what hid every write-shaped tool from the TUI
/// under `ask`. [`SessionUniverse::definitions`] consumes the approval
/// flag, never the stdin flag; the stdin fallback inside
/// `TerminalApproval::approve` consumes the stdin flag, never the approval
/// one.
pub(crate) fn approval_capabilities() -> (bool, bool) {
    (false, true)
}
