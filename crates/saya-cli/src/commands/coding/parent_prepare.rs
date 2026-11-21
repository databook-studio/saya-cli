//! Opening a run without a turn.
//!
//! A chat session opens its run while the human is still typing: the slot is
//! claimed, the run is composed and held, and the first message the user sends
//! is the run's FIRST TURN — `run_next_turn` — rather than a run of its own.
//! `run_open` stays what a job surface calls: the same open, immediately
//! followed by the task's turn, so its behaviour and its goldens are unchanged.
//!
//! The open is what both entries share, and it is stated once here: the
//! per-turn inputs a surface hands over, the one-time composition, and the hold
//! that keeps the provider, toolset, hooks, ledger, mailbox and lease for every
//! turn of the run. The job's `request.task` stays a real, stable label —
//! `"session <id>"` for a prepared run — because every helper brief and the
//! tool advertisement read it; it is never the first message.

use super::parent_turn::RootSession;
use super::parent_turn_limits::TurnInputs;
use super::{CodingError, NativeCodingJob};
use saya_agent::{AgentEventSink, AgentMailbox};
use std::sync::Arc;

impl NativeCodingJob {
    /// Opens the run and runs no turn.
    ///
    /// After it, the run is open: `live_runs::run_is_open` is true for this
    /// process, no provider request has been served, no turn has ended, and the
    /// holder of the folder lease is this job. The first message is
    /// `run_next_turn`, whose request carries exactly that message — the sink is
    /// stated here for the same surface's turns, and an open with no turn
    /// reports nothing on it.
    pub(crate) async fn run_prepare(
        self: &Arc<Self>,
        _sink: &dyn AgentEventSink,
        inputs: TurnInputs,
    ) -> Result<(), CodingError> {
        // The mailbox is taken once, before anything is composed: a second open
        // of the same run is `Busy` and touches nothing the first one owns.
        let mailbox = self
            .root_mailbox
            .lock()
            .await
            .take()
            .ok_or(CodingError::Busy)?;
        // The slot is held across the composition for the same reason: an open
        // is one exclusive use of the run, and a message that arrives while it
        // is still being set up is refused `Busy` rather than racing it.
        let turn = self.root_run.claim()?;
        self.open_root_session(mailbox, &inputs).await?;
        // The open is over; the run stays open for its turns.
        drop(turn);
        Ok(())
    }

    /// The open both entries share: the surface's per-turn limits, the run's
    /// one-time composition, and the hold that keeps it for every turn.
    ///
    /// The caller owns the claimed turn slot and the taken mailbox, so the open
    /// and whatever follows it on the same call are one exclusive use.
    pub(super) async fn open_root_session(
        self: &Arc<Self>,
        mailbox: AgentMailbox,
        inputs: &TurnInputs,
    ) -> Result<Arc<RootSession>, CodingError> {
        // The policy the surface states at open bounds the first turn exactly as
        // it bounds every turn after it. Stating none leaves the run's slot as a
        // surface set it, which is how a caller states one before opening.
        if inputs.policy.is_some() {
            self.root_run.set_turn_policy(inputs.policy);
        }
        let session = Arc::new(self.compose_root(mailbox, inputs).await?);
        self.root_run.hold(Arc::clone(&session));
        Ok(session)
    }

    /// Closes the run: after it, the run is no longer open in this process and a
    /// message that arrives late is refused as one on a finished run instead of
    /// becoming another turn.
    ///
    /// This is a surface's close of a run whose turns are done — the session's
    /// quit — and never a turn's own end: a cancelled or stopped turn leaves the
    /// run open, so the next message is another turn of it.
    pub(crate) async fn run_close(&self) {
        self.root_run.close();
    }
}
