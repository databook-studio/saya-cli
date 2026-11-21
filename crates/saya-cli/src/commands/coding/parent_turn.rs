//! One root turn on an open job.
//!
//! A run used to be one `run_root`: take the mailbox once, run one loop, finish
//! once. Phase 3 unit 3-A splits it, so a message that arrives after the
//! previous turn ended normally is another TURN of the same run — the same
//! epoch, ledger, lease, mailbox owner and cancellation token — instead of a
//! resume (a new epoch, fresh consent, a re-taken lease, a re-opened
//! workspace).
//!
//! `run()` stays one job, one root run: it opens the run, runs the task's turn
//! and closes it. The composition below is the run's one-time setup, held on
//! the job while the run is open so no turn repeats it.
//!
//! The run also keeps the turns it has completed: every turn after the first
//! replays them as its history, in chat's shape and under chat's bound
//! (`history`), so a conversation on an open job reads as it does in chat.

use super::parent_turn_limits::TurnInputs;
use super::{CodingError, NativeCodingJob};
use crate::agent::hooks::{HookApprovalGate, Hooks};
use crate::agent::tools::coding::RootTools;
use crate::commands::coding::TurnStop;
use saya_agent::{
    ActivePlugins, AgentEventSink, AgentMailbox, AgentOutput, CancellationToken, ToolExecutor,
    TurnPolicy,
};
use saya_harness::coding::worktree::VerifiedCheckout;
use std::sync::{Arc, Mutex};

#[path = "parent_history.rs"]
mod history;
pub(super) use history::TurnHistory;

/// Everything a root turn needs that is composed ONCE per run: the provider,
/// the toolset, the hooks over the run's own checkout, the executor that puts
/// the hooks before the tools, the approval gate, and the checkout the turn
/// checkpoints. A turn builds only what belongs to that turn — the sink chain,
/// the control, the progress handle and the request.
pub(super) struct RootSession {
    pub(super) provider: Box<dyn saya_agent::ChatProvider>,
    pub(super) tools: Arc<RootTools>,
    pub(super) hooks: Option<Arc<Hooks>>,
    /// A `PermissionRequest` hook can only refuse an ask the root would make.
    pub(super) approval_gate: Option<Arc<HookApprovalGate>>,
    pub(super) executor: Arc<dyn ToolExecutor>,
    /// The run's working checkout, resolved once: every checkpoint of the run
    /// is of this one directory.
    pub(super) checkout: Option<Arc<VerifiedCheckout>>,
    pub(super) system_prompt: Option<String>,
    pub(super) model: String,
    /// The run's mailbox. The first turn takes it, because the loop consumes
    /// the receiver; a later turn carries none, so steering a root between
    /// turns arrives nowhere until an attached front end owns it.
    pub(super) mailbox: Mutex<Option<AgentMailbox>>,
    /// The turns this run has completed, in order: what a further turn
    /// replays as its history, bounded by the run's own context byte budget
    /// exactly as chat bounds its own.
    pub(super) history: TurnHistory,
    /// The plugins this run activated at open, or `None` for a run that
    /// consults no registry — every job surface today.
    pub(super) plugins: Option<Arc<ActivePlugins>>,
    /// Whether this run's turns carry the project's untrusted context — its
    /// instruction sources and the worktree's explicit preferences. The chat
    /// surface does, as the line session's turn did on release; a job surface
    /// states its own context and carries none of it.
    pub(super) project_context: bool,
}

impl NativeCodingJob {
    /// Runs the run's first turn and leaves the run OPEN: the provider,
    /// toolset, hooks, ledger and — in place — lease stay held, so a message
    /// can be another turn of this run instead of a resume. `run()` is this
    /// plus the owner's end-of-run work and the one finish.
    #[allow(dead_code)] // the attached front end of a later unit is its caller
    pub(crate) async fn run_open(
        self: &Arc<Self>,
        sink: &dyn AgentEventSink,
        inputs: TurnInputs,
    ) -> Result<AgentOutput, CodingError> {
        let mailbox = self
            .root_mailbox
            .lock()
            .await
            .take()
            .ok_or(CodingError::Busy)?;
        self.open_root(sink, mailbox, inputs).await
    }

    /// The same, over a mailbox the run's own caller already took, so one run
    /// takes the mailbox exactly once.
    pub(super) async fn open_root(
        self: &Arc<Self>,
        sink: &dyn AgentEventSink,
        mailbox: AgentMailbox,
        inputs: TurnInputs,
    ) -> Result<AgentOutput, CodingError> {
        // The slot is claimed before the composition, so a message that
        // arrives while the first turn is still being set up is refused `Busy`
        // rather than starting a second turn beside it.
        let turn = self.root_run.claim()?;
        // The open the prepared run also takes: the surface's per-turn inputs,
        // the run's one-time setup and the hold that keeps it.
        let session = self.open_root_session(mailbox, &inputs).await?;
        let result = self
            .run_turn(&session, sink, self.request.task.clone())
            .await;
        drop(turn);
        result
    }

    /// Runs another turn on an open in-place job after the previous turn ended
    /// normally: the same epoch, lease, ledger and cancellation token, with no
    /// consent prompt, no lease hand-over and no workspace re-open. A turn
    /// that is already running is `Busy`; a finished run and a
    /// managed-checkout job are refused, in place being the only shape this
    /// release runs a further turn on.
    pub(crate) async fn run_next_turn(
        &self,
        sink: &dyn AgentEventSink,
        prompt: &str,
    ) -> Result<AgentOutput, CodingError> {
        let _turn = self.root_run.claim()?;
        if !self.application.is_in_place() {
            return Err(not_continuable(
                self,
                "works in a managed checkout, which runs one turn per run; a further turn needs an in-place job",
            ));
        }
        let Some(session) = self.root_run.session() else {
            return Err(not_continuable(
                self,
                "has already finished its run; a further turn needs a resumed job",
            ));
        };
        // This release has no attached front end, so the caller's sink is what
        // a further turn reports to; a job surface passes one with nothing
        // downstream, as it always did.
        self.run_turn(&session, sink, prompt.to_owned()).await
    }

    /// Sets the limits every turn of this run applies, or clears them.
    ///
    /// An explicit input a surface states before the turn it applies to:
    /// `None` — the default — is a turn with no turn-scoped budget, which is
    /// every job turn today.
    #[allow(dead_code)] // the chat surface of phase 4.6 is its production caller
    pub(crate) fn set_turn_policy(&self, policy: Option<TurnPolicy>) {
        self.root_run.set_turn_policy(policy);
    }

    /// How the turn that ended most recently ended, or `None` for one that
    /// simply finished.
    pub(crate) fn turn_stop(&self) -> Option<TurnStop> {
        self.root_run.turn_stop()
    }

    /// The stop signal of the turn running right now, or `None` between turns:
    /// what an attached surface cancels to end that turn and no other.
    pub(crate) fn turn_cancellation(&self) -> Option<CancellationToken> {
        self.root_run.turn_cancellation()
    }
}

/// The closed refusal a run that cannot take another turn states.
#[allow(dead_code)] // only `run_next_turn` names it, and that entry is not wired yet
fn not_continuable(job: &NativeCodingJob, detail: &str) -> CodingError {
    CodingError::NotResumable {
        job: job.job_id().as_str().to_owned(),
        detail: detail.to_owned(),
    }
}
