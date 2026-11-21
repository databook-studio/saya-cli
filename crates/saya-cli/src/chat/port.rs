//! What a session asks of the engine running its turns.
//!
//! A session knows four things about the run it drives: how to prepare it, how
//! to run the next turn, how to cancel one, and how to close it. Nothing above
//! that leaks in — no job record, no epoch, no slot — so the state machine can
//! be driven by a fake in a test and by the job engine in a session.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::AgentEventSink;

use super::ChatOutcome;

/// What a prepared run answers with: the profile it connected, when there is
/// one, and the workspace root it holds, when it opened one. A session passes
/// both to the front so the surface can say what it is talking to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Prepared {
    /// The connected profile's name.
    pub(crate) db: Option<String>,
    /// The workspace root, when the run opened one.
    pub(crate) workspace: Option<PathBuf>,
}

/// Why a turn did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PortError {
    /// A turn already holds the run's one slot. A session queues on this and
    /// says nothing about it: the operator asked for one thing at a time, not
    /// for a report on the engine's bookkeeping.
    Busy,
    /// The run refused the turn, in its own words.
    Failed(String),
}

/// The engine one session drives.
#[async_trait]
pub(crate) trait RunPort: Send + Sync {
    /// Composes the run and opens it. The error is the engine's own sentence,
    /// for the front to show; a session keeps the operator's next message
    /// rather than dropping it.
    async fn prepare(&self) -> Result<Prepared, String>;

    /// Runs one turn with the front's sink and returns how it ended. The prompt
    /// is the operator's own words, unmodified.
    async fn next_turn(
        &self,
        sink: Arc<dyn AgentEventSink>,
        prompt: String,
    ) -> Result<ChatOutcome, PortError>;

    /// Asks the turn in flight to stop. The turn still reports how it ended, as
    /// [`ChatOutcome::Cancelled`].
    async fn cancel_turn(&self);

    /// Finalizes the run: no turn is admitted afterwards.
    async fn close(&self);
}

/// How many times one message is offered to a port that answers `Busy`. The slot
/// is the session's own, so a busy answer only means the engine has not yet let
/// go of a turn it just ended: offering the message again is the whole remedy.
const BUSY_RETRIES: usize = 3;

/// What one turn produced.
#[derive(Debug)]
pub(crate) enum TurnOutcome {
    /// The turn ran, and ended this way.
    Ended(ChatOutcome),
    /// The slot stayed taken after every offer, so the message keeps its place.
    Busy(String),
    /// The run refused the turn, in its own words.
    Refused(String),
}

/// One turn, up to the port's slot. A port that answers `Busy` is offered the
/// message again rather than the operator being told about the engine's
/// bookkeeping.
pub(crate) async fn offer_turn(
    port: &dyn RunPort,
    sink: Arc<dyn AgentEventSink>,
    message: String,
) -> TurnOutcome {
    for _ in 0..BUSY_RETRIES {
        match port.next_turn(Arc::clone(&sink), message.clone()).await {
            Ok(outcome) => return TurnOutcome::Ended(outcome),
            Err(PortError::Busy) => tokio::task::yield_now().await,
            Err(PortError::Failed(refusal)) => return TurnOutcome::Refused(refusal),
        }
    }
    TurnOutcome::Busy(message)
}
