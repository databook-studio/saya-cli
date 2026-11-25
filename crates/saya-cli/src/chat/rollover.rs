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
use super::workspace_shape::decide as workspace_shape;
use crate::commands::coding::{CodingApplication, CodingError, NativeCodingJob};
use crate::interactive::session::continuation::compaction::manual::COMPACT_KEEP_TAIL_TURNS;

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
    /// The conversation was compacted: its older turns were folded into a
    /// summary, and the next run starts from that summary and the newest tail.
    Compact,
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

    /// Whether the change folds the conversation into a summary. A compaction
    /// leaves the transcript alone — what was said does not change — but the
    /// run it prepares carries the summary and only the newest tail pairs, so
    /// the turns the summary replaced are not replayed over it.
    pub(crate) fn compacts(self) -> bool {
        matches!(self, Self::Compact)
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
    run: End,
}

/// The run a rollover replaces, as the surface holds it: the composition a
/// session stated, or the job alone — a run the front resumed has no
/// composition of its own, the engine handed its job back already opened.
enum End {
    Composed(Box<RunComposition>),
    Adopted(Arc<NativeCodingJob>),
}

impl Ending {
    /// The session's run, handed over to the rollover that replaces it.
    pub(crate) fn new(run: RunComposition) -> Self {
        Self {
            run: End::Composed(Box::new(run)),
        }
    }

    /// A resumed run, handed over the same way.
    pub(crate) fn adopted(job: Arc<NativeCodingJob>) -> Self {
        Self {
            run: End::Adopted(job),
        }
    }

    /// The job every turn of the run named: what the rollover finalizes before
    /// the next job is prepared.
    fn job(&self) -> &Arc<NativeCodingJob> {
        match &self.run {
            End::Composed(run) => run.job(),
            End::Adopted(job) => job,
        }
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
    /// exactly as the first one was. The folder's shape is read after the run
    /// that ended has let it go: free, the successor writes it; held by a job
    /// that run started, the successor reads it.
    pub(crate) async fn roll(
        &self,
        ending: Ending,
        change: RolloverCause,
        mut next: SessionRun,
        transcript: &mut Transcript,
    ) -> Result<RunComposition, RolloverError> {
        let job = ending.job();
        // Whether this run is the folder's holder: dropping it is what frees
        // the folder, or hands it to a job it started that still runs.
        let held = job.holds_folder_lease();
        // A rollover prepares a new job: the run it replaces is over, and the
        // surface states the job that follows it.
        if job.job_id().as_str() == next.session {
            return Err(RolloverError::SameJob(next.session));
        }
        // The run's own last answer, as the surface recorded it, is the answer
        // its terminal record states.
        let answer = transcript.turns().last().map(|turn| turn.answer.clone());
        job.run_finish(answer)
            .await
            .map_err(RolloverError::Engine)?;
        // The run that ended gave its folder up as it closed — `run_finish`
        // hands it to a dependent job still running, or frees it — so the next
        // job is created after that and never before it.
        drop(ending);
        // The shape stated when the successor was planned is stale now: the run
        // that ended has let the folder go, and the successor must read a
        // folder a running job took rather than try to write beside it.
        if held {
            next.shape = workspace_shape(
                self.application.root_path(),
                self.application.state_root(),
                &next.session,
            );
        }
        // `/clear` starts the next run with nothing; every other change carries
        // the conversation over as the run's history.
        if change.clears_transcript() {
            transcript.clear();
        }
        let rolled = RunComposition::compose_seeded(
            Arc::clone(&self.application),
            next,
            seeded(transcript, change),
        )
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

/// The assistant's line of the summary pair: the run's history is (prompt,
/// answer) pairs by contract — `saya_agent`'s assembly refuses an odd length and
/// any role out of turn — so a summary standing ahead of the tail rides as one
/// pair rather than one message.
pub(super) const SUMMARY_ACK: &str = "Understood. I have the summary of the earlier conversation.";

/// The conversation a run is seeded with, oldest pair first: the transcript's
/// settled pairs and, once a compaction saved a summary, that summary ahead of
/// them — a compaction keeps the newest tail pairs, `/clear` keeps none.
fn seeded(transcript: &Transcript, change: RolloverCause) -> Vec<(String, String)> {
    if change.clears_transcript() {
        return Vec::new();
    }
    let turns = transcript.turns();
    let kept = if change.compacts() {
        turns.len().saturating_sub(COMPACT_KEEP_TAIL_TURNS)
    } else {
        0
    };
    let summary = transcript
        .record()
        .compaction_summary
        .clone()
        .map(|summary| (summary, SUMMARY_ACK.to_owned()));
    summary
        .into_iter()
        .chain(
            turns
                .into_iter()
                .skip(kept)
                .map(|turn| (turn.prompt, turn.answer)),
        )
        .collect()
}
