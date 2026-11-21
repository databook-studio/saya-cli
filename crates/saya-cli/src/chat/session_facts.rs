//! The session facts a chat run's requests carry: the compaction narrative the
//! loop's session state holds, and the continuation record beside it.
//!
//! A session outlives the run behind it, so the facts a request needs are the
//! session's, not the run's: they change when the chat layer compacts (a new
//! summary and a new record), when `/notes` edits the record, and when a resume
//! restores one from disk. One cell states them — written at the line front's
//! compaction boundary and read where a request is assembled — so the two
//! renderers the interactive turn already uses are reused, never copied.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_harness::workspace::Workspace;
use saya_store::StoredContinuation;

use crate::interactive::session::continuation::compaction::manual::CompactionPlan;
use crate::interactive::session::continuation::{render_record_block, render_summary_block};
use crate::interactive::session::state::SessionState;

/// The facts one request renders, as the session states them now.
#[derive(Default)]
struct Facts {
    /// The compaction narrative — the saved summary, under the state's own
    /// replay gate, worded the way the state words it.
    narrative: Option<String>,
    /// The session's continuation record, cloned once per refresh.
    continuation: Option<StoredContinuation>,
    /// A compaction the boundary performed and the loop has not applied yet:
    /// the plan it validated and the summary it saved.
    pending: Option<Pending>,
}

/// One compaction, as the boundary stated it: what the release's writer needs
/// to set the session's facts — the plan's own turn count and pinned tokens —
/// and the summary itself.
struct Pending {
    plan: CompactionPlan,
    summary: String,
}

/// The session's facts, shared by every run of the session.
pub(crate) struct SessionFacts {
    /// The workspace the session works over: the record renderer reads it to
    /// say whether an observed file changed since.
    workspace: Option<Arc<Workspace>>,
    facts: Mutex<Facts>,
}

impl SessionFacts {
    /// The facts a session's state holds now, over the workspace it works in.
    pub(crate) fn of(state: &SessionState, workspace: Option<Arc<Workspace>>) -> Arc<Self> {
        let facts = Self {
            workspace,
            facts: Mutex::new(Facts::default()),
        };
        facts.refresh(state);
        Arc::new(facts)
    }

    /// Re-reads the cell from the loop's state: the summary a compaction saved,
    /// the record `/notes` edited, and the one a resume restored all arrive
    /// here, so the next request states the session as it is now.
    pub(crate) fn refresh(&self, state: &SessionState) {
        let mut facts = self.locked();
        facts.narrative = state.compaction_narrative();
        facts.continuation = state.continuation.clone();
    }

    /// States the compaction the boundary performed: the plan it validated and
    /// the summary it saved. The loop applies it to its own state, through the
    /// one writer every compaction already used.
    pub(crate) fn note_compaction(&self, plan: &CompactionPlan, summary: &str) {
        self.locked().pending = Some(Pending {
            plan: plan.clone(),
            summary: summary.to_owned(),
        });
    }

    /// Applies the compaction the boundary stated to the loop's state — through
    /// the release's one writer — and re-reads the cell from it. A compaction
    /// the boundary never stated, or one whose record cannot be assembled,
    /// leaves the state's own facts exactly as they were.
    pub(crate) fn apply(
        &self,
        state: &mut SessionState,
        digest_of: &dyn Fn(&str) -> Option<String>,
    ) {
        let pending = self.locked().pending.take();
        if let Some(pending) = pending {
            let _ = crate::interactive::session::continuation::compaction::manual::apply_with_continuation(
                state,
                &pending.plan,
                &pending.summary,
                digest_of,
            );
        }
        self.refresh(state);
    }

    /// The blocks this request carries, through the session's own renderers:
    /// the compaction narrative first, then the continuation record, each cut
    /// to the run's context byte budget.
    pub(crate) fn blocks(&self, context_byte_budget: usize) -> Vec<saya_agent::ContextBlock> {
        let facts = self.locked();
        let mut blocks = Vec::new();
        if let Some(narrative) = facts.narrative.as_deref()
            && let Some(block) = render_summary_block(narrative, context_byte_budget)
        {
            blocks.push(block);
        }
        if let Some(block) = render_record_block(
            facts.continuation.as_ref(),
            self.workspace.as_deref(),
            context_byte_budget,
        ) {
            blocks.push(block);
        }
        blocks
    }

    fn locked(&self) -> MutexGuard<'_, Facts> {
        self.facts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
