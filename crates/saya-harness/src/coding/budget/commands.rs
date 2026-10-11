//! Command effect budget lifecycle for the coding ledger. Commands bypass
//! the agent loop like child sessions: `begin_command`/`finish_command`
//! live outside the `EffectAdmission` trait. A `CommandPermit` dropped
//! without settlement stays held as Unknown by design (the saya-agent
//! `coding_admission.rs` contract).

use std::{fmt, num::NonZeroU64, path::PathBuf};

use crate::coding::commands::{
    CommandOwner, CommandSession, CommandSessionError, CommandStartError,
};
use crate::runner::{ProgramOutcome, RunProgram};

use super::ledger::JobBudgetLedger;
use saya_agent::{EffectAdmissionError, EffectOutcome, EffectPermit};
use saya_types::BudgetDimension;

/// An admitted command slot bound to the ledger that admitted it: settling it on any other ledger is an owner failure, as for child sessions.
#[derive(Clone)]
pub struct CommandPermit {
    permit: EffectPermit,
    owner: std::sync::Arc<tokio::sync::Mutex<super::ledger::State>>,
}

impl JobBudgetLedger {
    pub async fn begin_command(&self) -> Result<CommandPermit, EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        let permit = state
            .admit_one(BudgetDimension::CommandSlots, 1, None, None)
            .await?;
        Ok(CommandPermit {
            permit,
            owner: self.inner.clone(),
        })
    }

    pub async fn finish_command(
        &self,
        permit: CommandPermit,
        outcome: EffectOutcome,
    ) -> Result<(), EffectAdmissionError> {
        let mut state = self.inner.lock().await;
        if !std::sync::Arc::ptr_eq(&permit.owner, &self.inner) {
            return Err(state.owner_failure());
        }
        state
            .finish(
                permit.permit,
                super::settlement::Settlement::Command(outcome),
            )
            .await?;
        if outcome != EffectOutcome::Unknown {
            state.counts.command_slots = state
                .counts
                .command_slots
                .checked_sub(1)
                .ok_or_else(|| state.owner_failure())?;
        }
        Ok(())
    }
}

/// Why a budgeted command start failed: admission precedes the owner start,
/// so a refusal distinguishes a ledger denial from an owner refusal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BudgetedCommandStartError {
    /// The ledger refused to admit the command slot.
    #[error("command budget admission failed")]
    Budget(#[source] EffectAdmissionError),
    /// The command owner refused to start the command.
    #[error("command start refused")]
    Start(#[source] CommandStartError),
}

/// The joined result of a budgeted command and its one budget settlement.
#[derive(Debug)]
pub struct BudgetedJoin {
    /// The underlying session's `join`/`stop_and_join` result.
    pub command: Result<ProgramOutcome, CommandSessionError>,
    /// The single settlement of the command permit against the ledger.
    pub settlement: Result<(), EffectAdmissionError>,
}

/// A started command wrapped with its admitted permit and its ledger.
///
/// All non-consuming session methods are reached through [`Self::session`].
/// [`Self::join`] and [`Self::stop_and_join`] consume the wrapper and settle
/// the permit exactly once. Dropping the wrapper without joining does NOT
/// settle: the permit stays held as Unknown by design, so there is no `Drop`
/// impl.
pub struct BudgetedCommandSession {
    session: CommandSession,
    permit: CommandPermit,
    ledger: JobBudgetLedger,
}

impl fmt::Debug for BudgetedCommandSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BudgetedCommandSession")
            .field("is_finished", &self.session.is_finished())
            .finish()
    }
}

async fn settle_refused_start(
    ledger: &JobBudgetLedger,
    permit: CommandPermit,
    start_error: CommandStartError,
) -> BudgetedCommandStartError {
    if let Err(admission) = ledger
        .finish_command(permit, EffectOutcome::NotStarted)
        .await
    {
        return BudgetedCommandStartError::Budget(admission);
    }
    BudgetedCommandStartError::Start(start_error)
}

/// Admits a command slot, then starts the command without stdin control.
pub async fn start_budgeted(
    ledger: &JobBudgetLedger,
    owner: &CommandOwner,
    runner: RunProgram,
    program: String,
    argv: Vec<String>,
    timeout_seconds: NonZeroU64,
) -> Result<BudgetedCommandSession, BudgetedCommandStartError> {
    let permit = ledger
        .begin_command()
        .await
        .map_err(BudgetedCommandStartError::Budget)?;
    match owner.start(runner, program, argv, timeout_seconds) {
        Ok(session) => Ok(BudgetedCommandSession {
            session,
            permit,
            ledger: ledger.clone(),
        }),
        Err(error) => Err(settle_refused_start(ledger, permit, error).await),
    }
}

/// Admits a command slot, then starts the command with a bounded stdin queue
/// and explicit EOF.
pub async fn start_budgeted_with_stdin(
    ledger: &JobBudgetLedger,
    owner: &CommandOwner,
    runner: RunProgram,
    program: String,
    argv: Vec<String>,
    timeout_seconds: NonZeroU64,
) -> Result<BudgetedCommandSession, BudgetedCommandStartError> {
    let permit = ledger
        .begin_command()
        .await
        .map_err(BudgetedCommandStartError::Budget)?;
    match owner.start_with_stdin(runner, program, argv, timeout_seconds) {
        Ok(session) => Ok(BudgetedCommandSession {
            session,
            permit,
            ledger: ledger.clone(),
        }),
        Err(error) => Err(settle_refused_start(ledger, permit, error).await),
    }
}

/// Admits a command slot, then starts the command appending redacted
/// captured output to a private bounded store under `store_root`.
pub async fn start_budgeted_with_output(
    ledger: &JobBudgetLedger,
    owner: &CommandOwner,
    runner: RunProgram,
    program: String,
    argv: Vec<String>,
    timeout_seconds: NonZeroU64,
    store_root: PathBuf,
) -> Result<BudgetedCommandSession, BudgetedCommandStartError> {
    let permit = ledger
        .begin_command()
        .await
        .map_err(BudgetedCommandStartError::Budget)?;
    match owner.start_with_output(runner, program, argv, timeout_seconds, store_root) {
        Ok(session) => Ok(BudgetedCommandSession {
            session,
            permit,
            ledger: ledger.clone(),
        }),
        Err(error) => Err(settle_refused_start(ledger, permit, error).await),
    }
}

impl BudgetedCommandSession {
    /// The wrapped session; every non-consuming session method is reached
    /// through it.
    pub fn session(&self) -> &CommandSession {
        &self.session
    }

    /// Waits for the command outcome, then settles the permit exactly once
    /// with the mapped [`EffectOutcome`].
    pub async fn join(self) -> BudgetedJoin {
        let Self {
            session,
            permit,
            ledger,
        } = self;
        let command = session.join().await;
        let settlement = ledger
            .finish_command(permit, command_outcome(&command))
            .await;
        BudgetedJoin {
            command,
            settlement,
        }
    }

    /// Stops the command, waits for its outcome after cleanup, then settles
    /// the permit exactly once with the mapped [`EffectOutcome`].
    pub async fn stop_and_join(self) -> BudgetedJoin {
        let Self {
            session,
            permit,
            ledger,
        } = self;
        let command = session.stop_and_join().await;
        let settlement = ledger
            .finish_command(permit, command_outcome(&command))
            .await;
        BudgetedJoin {
            command,
            settlement,
        }
    }
}

/// Maps a joined command result to the budget settlement outcome.
fn command_outcome(command: &Result<ProgramOutcome, CommandSessionError>) -> EffectOutcome {
    match command {
        Ok(_) => EffectOutcome::Succeeded,
        // D3: the embedded outcome proves the process spawned and was reaped.
        Err(CommandSessionError::InputIncomplete { .. })
        | Err(CommandSessionError::InputWriteFailed { .. }) => EffectOutcome::Succeeded,
        Err(CommandSessionError::CancelledBeforeStart)
        // The owner requested the stop and the supervision task was joined, so
        // a runner refusal that happened before any process existed is a known
        // not-started terminal state, never unknown (`StoppedBeforeSpawn` is
        // produced only by `stop_and_join`). Unknown stays reserved for
        // genuinely unknown outcomes: a natural join of a failed session (the
        // caller never asked to stop it) and a runner failure after the
        // process existed (the child may outlive the wait), plus supervision
        // join failures.
        | Err(CommandSessionError::StoppedBeforeSpawn { .. }) => EffectOutcome::NotStarted,
        Err(CommandSessionError::Runner(_)) | Err(CommandSessionError::Join(_)) => {
            EffectOutcome::Unknown
        } // Exhaustive on purpose: a future CommandSessionError variant must choose its settlement here (conservatively Unknown) instead of silently defaulting.
    }
}
