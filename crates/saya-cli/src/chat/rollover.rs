//! A context change rolls the session's run over.
//!
//! Changing what a session talks to — its model, its provider, its connection or
//! profile, its privacy setting — or clearing the conversation is a change the
//! run already open cannot absorb: the run's request, its prompt prefix and its
//! provider were composed once, at open. So the change ends the run and prepares
//! the next one: run n is closed and finalized, and run n+1 opens with the
//! conversation carried over as its seed, under a link that names it.
//!
//! Two rules decide the order and the shape:
//!
//! - **A rollover prepares a new job.** Resuming the job it replaces would
//!   re-take the folder's lease inside this process, which contends with the run
//!   that still holds it, and would ask for a fresh approval every time. A new
//!   job is the simpler shape: no second consent (the session's mode is the
//!   consent), no second lease, and the link moves to the job the surface named.
//! - **A rollover never races a turn.** While a turn runs the change waits: it
//!   is recorded now and rolled after the turn ends, so the answer the operator
//!   just read is part of the conversation the next run is seeded with. A
//!   requested change is never dropped.
//!
//! The lease is why the closing run is handed over rather than lent: the roll
//! states its close, finalizes it and drops it — the job object owns the folder
//! lease — before the next job is created.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::compose::{RunComposition, SessionRun};
use super::link;
use super::transcript::Transcript;
use crate::commands::coding::{CodingApplication, CodingError};

/// What a context change changed. The change is what the surface states; the
/// run it states next is composed from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RolloverCause {
    /// A different model.
    Model,
    /// A different provider, or the same one at a different endpoint.
    Provider,
    /// A different connection, or a different profile's connections.
    Connection,
    /// A different data-sharing setting.
    Privacy,
    /// The conversation cleared: the next run starts with nothing.
    Clear,
}

impl RolloverCause {
    /// Whether the change clears the conversation. `/clear` is the one change
    /// that starts the next run with an empty seed and an empty transcript;
    /// every other change carries the conversation over.
    pub(crate) fn clears_transcript(self) -> bool {
        matches!(self, Self::Clear)
    }
}

/// Why a rollover did not prepare the next run.
#[derive(Debug)]
pub(crate) enum RolloverError {
    /// A rollover prepares a new job: the next run's job id must differ from the
    /// one it replaces, or the rollover would re-open the run it just closed.
    SameJob(String),
    /// The engine refused the next run, in its own words.
    Engine(CodingError),
}

impl std::fmt::Display for RolloverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SameJob(job) => write!(
                formatter,
                "a rollover prepares a new job, and this one still names `{job}`"
            ),
            Self::Engine(error) => write!(formatter, "{error}"),
        }
    }
}

/// The run a rollover ends: the session's composed run, handed over.
///
/// Consuming it is what makes the close stated once — the roll takes it by
/// value, closes and finalizes it, and drops it so the folder lease is free
/// before the next job is created.
pub(crate) struct Ending {
    run: RunComposition,
}

impl Ending {
    /// The session's run, handed over to the rollover that replaces it.
    pub(crate) fn new(run: RunComposition) -> Self {
        Self { run }
    }
}

/// A change waiting for the turn in flight to end.
struct Waiting {
    change: RolloverCause,
    next: SessionRun,
}

/// The session's rollovers: the change waiting for a turn to end, and the one
/// step that ends a run and prepares the next.
pub(crate) struct Rollover {
    /// The application every run of the session is composed over.
    application: Arc<CodingApplication>,
    /// The change recorded while a turn runs, taken by the roll that follows it.
    waiting: Mutex<Option<Waiting>>,
}

impl Rollover {
    /// The rollover of one session's runs, over the application they open on.
    pub(crate) fn new(application: Arc<CodingApplication>) -> Self {
        Self {
            application,
            waiting: Mutex::new(None),
        }
    }

    /// Records the change a running turn holds back: it is rolled when the turn
    /// ends, and a second change replaces it — the newest statement of where the
    /// session should point is the one that rolls.
    pub(crate) fn request(&self, change: RolloverCause, next: SessionRun) {
        *self.queue() = Some(Waiting { change, next });
    }

    /// Whether a change is waiting for a turn to end.
    pub(crate) fn waiting(&self) -> bool {
        self.queue().is_some()
    }

    /// Rolls now: the run handed over is closed and finalized, dropped, and the
    /// run `next` states is prepared from it — seeded with the conversation
    /// unless the change is `/clear`, and linked into the record.
    ///
    /// **Nothing is asked.** The next run is consented to by the session's mode,
    /// exactly as the first one was, and the in-place lease of the run that
    /// ended is free before the next job is created.
    pub(crate) async fn roll(
        &self,
        ending: Ending,
        change: RolloverCause,
        next: SessionRun,
        transcript: &mut Transcript,
    ) -> Result<RunComposition, RolloverError> {
        let run = ending.run;
        // A rollover prepares a new job: the run it replaces is over, and the
        // surface states the job that follows it.
        if run.job().job_id().as_str() == next.session {
            return Err(RolloverError::SameJob(next.session));
        }
        // The run's own last answer, as the surface recorded it, is the answer
        // its terminal record states.
        let answer = transcript.turns().last().map(|turn| turn.answer.clone());
        run.job()
            .run_finish(answer)
            .await
            .map_err(RolloverError::Engine)?;
        // The job object owns the folder's lease, in place: dropping the run
        // that ended is what frees it, and the next job is created after that
        // and never before it.
        drop(run);
        // `/clear` starts the next run with nothing; every other change carries
        // the conversation over as the run's history.
        if change.clears_transcript() {
            transcript.clear();
        }
        let rolled =
            RunComposition::compose_seeded(Arc::clone(&self.application), next, seeded(transcript))
                .await
                .map_err(RolloverError::Engine)?;
        rolled.open().await.map_err(RolloverError::Engine)?;
        // The record names the run the session belongs to now.
        link::after_rollover(
            transcript.record_mut(),
            rolled.job().job_id(),
            rolled.job().epoch(),
        );
        Ok(rolled)
    }

    /// Rolls the change a turn held back, once that turn has ended: `Ok(None)`
    /// when no change is waiting.
    pub(crate) async fn after_turn(
        &self,
        ending: Ending,
        transcript: &mut Transcript,
    ) -> Result<Option<RunComposition>, RolloverError> {
        let Some(waiting) = self.queue().take() else {
            return Ok(None);
        };
        self.roll(ending, waiting.change, waiting.next, transcript)
            .await
            .map(Some)
    }

    fn queue(&self) -> MutexGuard<'_, Option<Waiting>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The conversation a run is seeded with: the transcript's settled pairs,
/// oldest first. Empty after `/clear` and for a session that settled none.
fn seeded(transcript: &Transcript) -> Vec<(String, String)> {
    transcript
        .turns()
        .into_iter()
        .map(|turn| (turn.prompt, turn.answer))
        .collect()
}
