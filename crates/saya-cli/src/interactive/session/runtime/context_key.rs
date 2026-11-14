//! The one key that says whether a slash command changed the session's
//! answering context. Derived only from existing `SessionState` fields — no
//! new persisted field, no epoch, no handle — so a saved session is
//! byte-identical before and after this module existed. Both dispatch sites
//! (headless loop, TUI) snapshot the key around `SessionState::apply` and
//! hand the pair to `abandon_question_when_context_changed`.

use super::SessionRuntime;
use crate::interactive::session::state::SessionState;

/// What the model's answering context is made of: the selected profile, the
/// included profiles (`/exclude` removes from this same set), the provider,
/// the model, privacy, the approval policy, the task posture, and the shape
/// of the cleared collections (conversation lengths, the compaction marker,
/// the task count) that `/clear` and `/tasks clear` reset. Equality over
/// these separates a real mutation from a usage line or a refused change;
/// display and accounting state the context does not answer through (the
/// thinking toggle, the usage accumulator, the warn flags) is deliberately
/// not part of the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionContextKey {
    profile: Option<String>,
    included_profiles: Vec<String>,
    provider: String,
    model: String,
    allow_data_sharing: bool,
    approval_mode: String,
    agent_mode: String,
    messages_len: usize,
    turns_len: usize,
    compacted_turns: usize,
    has_compaction_summary: bool,
    task_count: usize,
}

impl SessionState {
    /// The answering context as it stands right now. Cheap enough to take
    /// twice around one user-typed command: a few small clones beside the
    /// lengths and booleans the cleared collections already carry.
    pub(crate) fn context_key(&self) -> SessionContextKey {
        SessionContextKey {
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            allow_data_sharing: self.allow_data_sharing,
            approval_mode: self.approval_mode.clone(),
            agent_mode: self.agent_mode.clone(),
            messages_len: self.messages.len(),
            turns_len: self.turns.len(),
            compacted_turns: self.compacted_turns,
            has_compaction_summary: self.compaction_summary.is_some(),
            task_count: self.task_list.tasks.len(),
        }
    }

    /// The one abandon step both surfaces share: when the key moved across
    /// the command, the old question authority is stale — cancel its token
    /// and go Idle, so the next question mints a fresh scope. An unchanged
    /// key touches nothing, and a pending clarification survives. Both
    /// surfaces call this after applying the command, in their existing
    /// message and event order.
    pub(crate) fn abandon_question_when_context_changed(
        &self,
        before: &SessionContextKey,
        session: &mut SessionRuntime,
    ) {
        if *before != self.context_key() {
            session.abandon_question();
        }
    }
}
