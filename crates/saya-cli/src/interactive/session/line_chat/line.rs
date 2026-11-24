//! One line session: the run it is on, and what a turn of it changes.
//!
//! The surface above ([`super::LineChat`]) hands this one line at a time and
//! knows nothing below it: the run it is on, the turns that settled in the
//! session's transcript, and the one compaction every turn's boundary runs
//! ([`super::boundary`]). A rollover replaces the run — the successor is opened
//! before the next line, so the line after a compaction runs on the new job,
//! seeded with the summary and the kept tail.

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_store::{RedactedSession, RunLink};

use super::boundary::{self, Bounded, Said, Turn};
use super::open::{LineSession, open_run};
use super::run::{Next, Run, Sent};
use crate::agent::tools::shell_policy::ModeCell;
use crate::agent::turn_config::PromptOverrides;
use crate::chat::{
    ChatFront, ChatOutcome, ChatSession, Compaction, Rollover, RunComposition, RunPort, Transcript,
    WorkspaceShape,
};
use crate::commands::coding::CodingApplication;
use crate::front::LineFront;
use crate::interactive::turn_summary::ToolTally;
use crate::local_metrics::LocalMetrics;

/// What a line with no run left says.
const ENDED: &str = "the session's run has ended: its replacement could not be prepared";

/// What a turn no terminal front drives says: the surface reads its own turn
/// and states it ([`Line::settled`]).
const SURFACE_DRIVES: &str = "this session's turns are driven by the surface that holds it";

/// One line session: the run it is on, the turns it settled, and the one
/// compaction every turn crosses.
pub(crate) struct Line {
    /// The facts every run is composed from, under a new job after a rollover.
    parts: LineSession,
    /// The surface every run of this session composes its turns on: the
    /// terminal's own for the line loop, the full-screen surface's for the other.
    front: Arc<dyn ChatFront>,
    /// The terminal front a line turn's outcome and words are read from. The
    /// line loop drives its turns itself and states it ([`Line::send`]); the
    /// full-screen surface reads its own queue and states what it read
    /// ([`Line::settled`]), so it states none here.
    reads: Option<Arc<LineFront>>,
    /// What a run of this folder may write: the session's own fact, taken
    /// before the first run held the lease, and stated again by every
    /// successor.
    shape: WorkspaceShape,
    /// The run's own counters, when the session has a scope to record into.
    metrics: Option<Arc<LocalMetrics>>,
    /// The run the session is on. `None` once a rollover could not prepare its
    /// successor: the run it replaced is already closed, so no line runs.
    run: Option<Run>,
    /// The turns that settled: what a compaction plans over.
    transcript: Transcript,
    /// The session's compaction, and the latch a failure leaves behind, and the
    /// rollover it asks for.
    compaction: Compaction,
    rollover: Rollover,
    /// How many runs this session has opened: the next job's name.
    opened: usize,
    /// How many turns this session has settled: what a state change before the
    /// first one gives the run up over.
    settled: usize,
}

impl Line {
    /// The session's first run, open over its folder and already prepared.
    pub(super) fn new(
        application: Arc<CodingApplication>,
        parts: LineSession,
        front: Arc<LineFront>,
        shape: WorkspaceShape,
        metrics: Option<Arc<LocalMetrics>>,
        run: Run,
    ) -> Self {
        // The record the conversation rides: the session's id and model.
        let transcript = Transcript::from_record(RedactedSession {
            id: parts.session.clone(),
            model: run.model(),
            ..RedactedSession::default()
        });
        // The session's own facts, stated before the parts are taken: a
        // compaction this front runs writes them, and its turns read them.
        let compaction = Compaction::new().stating(parts.facts.clone());
        Self {
            rollover: Rollover::new(application),
            parts,
            front: Arc::clone(&front) as Arc<dyn ChatFront>,
            reads: Some(front),
            shape,
            metrics,
            run: Some(run),
            transcript,
            compaction,
            opened: 1,
            settled: 0,
        }
    }

    /// The same session, held by a surface that drives its own turns: the run the
    /// launch opened, over the surface's own front. The surface sends a turn and
    /// states how it settled ([`Line::settled`]), so no terminal front is read
    /// here; everything else — the transcript, the compaction, the rollover — is
    /// the line loop's own.
    ///
    /// `record` is the record the run was opened for, so the transcript a
    /// compaction plans over is the conversation the session already holds and
    /// not an empty one: a resumed session's turns are its own from the first
    /// frame.
    pub(crate) fn surface(
        application: Arc<CodingApplication>,
        parts: LineSession,
        front: Arc<dyn ChatFront>,
        shape: WorkspaceShape,
        metrics: Option<Arc<LocalMetrics>>,
        run: Run,
        record: &saya_store::RedactedSession,
    ) -> Self {
        let transcript = Transcript::from_record(record.clone());
        let compaction = Compaction::new().stating(parts.facts.clone());
        Self {
            rollover: Rollover::new(application),
            parts,
            front,
            reads: None,
            shape,
            metrics,
            run: Some(run),
            transcript,
            compaction,
            opened: 1,
            settled: 0,
        }
    }

    /// The session facts this front's compaction states and its requests are
    /// rendered from: the one cell the loop applies to its session state.
    pub(super) fn facts(&self) -> Option<std::sync::Arc<crate::chat::SessionFacts>> {
        self.parts.facts.clone()
    }

    /// The link the record carries to the run the session belongs to now.
    pub(super) fn run_link(&self) -> Option<RunLink> {
        self.run.as_ref().and_then(Run::link)
    }

    /// The tool tally of every turn this session ran.
    pub(super) fn tally(&self) -> ToolTally {
        match self.reads.as_ref() {
            Some(front) => front.tally(),
            // A surface that drives its own turns states its own tally: none of
            // the line loop's is read on its behalf.
            None => ToolTally::default(),
        }
    }

    /// Ends the run: the session's own quit, stated once.
    pub(super) async fn quit(&self) {
        if let Some(run) = self.run.as_ref() {
            run.session().quit().await.ok();
        }
    }

    /// The run this session is on, for the give-up that ends a prepared one.
    pub(super) fn run(&self) -> Option<&Run> {
        self.run.as_ref()
    }

    /// The turn's boundary, worded for the automatic trigger.
    async fn boundary(
        &mut self,
        outcome: &ChatOutcome,
        output: Option<saya_agent::AgentOutput>,
        prompt: &str,
        answer: &str,
    ) -> Said {
        let Some(next) = self.next() else {
            return Said::Nothing;
        };
        let turn = Turn {
            outcome,
            prompt,
            answer,
            output,
        };
        let bounded = boundary::after_turn(
            &self.compaction,
            &self.rollover,
            &mut self.run,
            &mut self.transcript,
            &turn,
            || next.compose(),
        )
        .await;
        self.take(bounded).await
    }

    /// Records what a boundary did: the successor replaces the run, or the run
    /// that ended is gone.
    async fn take(&mut self, bounded: Bounded) -> Said {
        match bounded {
            Bounded::Kept(said) => said,
            Bounded::Rolled { successor, said } => {
                self.replaced(*successor).await;
                said
            }
            Bounded::Ended(said) => {
                self.run = None;
                said
            }
        }
    }

    /// The run a rollover prepared: a chat session bound to the new job, with
    /// the same counters the old one had.
    async fn replaced(&mut self, successor: RunComposition) {
        let port = Arc::new(match self.metrics.clone() {
            Some(metrics) => {
                crate::chat::JobPort::opened(Arc::clone(successor.job())).with_metrics(metrics)
            }
            None => crate::chat::JobPort::opened(Arc::clone(successor.job())),
        });
        let session = ChatSession::new(
            Arc::clone(&port) as Arc<dyn RunPort>,
            Arc::clone(&self.front) as Arc<dyn ChatFront>,
        );
        session.open().await;
        self.opened += 1;
        self.run = Some(Run::successor(successor, session, port));
    }

    /// The next run's composer: the session's own facts, under the next name.
    fn next(&self) -> Option<Next> {
        self.run.as_ref()?;
        Some(Next::new(
            self.next_name(),
            Arc::clone(&self.front) as Arc<dyn ChatFront>,
            self.parts.clone(),
            self.shape.clone(),
        ))
    }

    /// The next job's name: the session's own, numbered — an id is
    /// `[A-Za-z0-9_-]` up to 128 bytes, so a long session name is cut to fit.
    fn next_name(&self) -> String {
        let suffix = format!("-{}", self.opened + 1);
        let keep = self.parts.session.len().min(128 - suffix.len());
        format!("{}{suffix}", &self.parts.session[..keep])
    }
}

// What a surface that drives its own turns reads and moves: the run it is on
// now, and the boundary a turn of it crosses (`surface.rs`).
include!("surface.rs");
