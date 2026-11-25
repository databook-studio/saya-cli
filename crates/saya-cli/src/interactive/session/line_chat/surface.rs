// The surface's half of one line session: the run it is on now, the boundary a
// turn of it crosses when the surface's own loop drove that turn, and the
// compaction and clear a surface states on the live run (`line.rs`).
//
// The full-screen surface holds the same `Line` the line loop drives — the
// transcript, the compaction and the rollover — and drives its own turn through
// it, so `/clear`, `/compact` and `/rotate` perform the line front's own
// operations on the live run rather than on the run that follows. What differs
// is only who reads the settled turn: the line loop reads its terminal front
// ([`Line::send`], `surface_send.rs`), and this surface reads its own queue
// ([`Line::settled`]). The run a state change gives up is `restate.rs`, and the
// session a `/resume` opens in place of this one is `surface_resume.rs`.

impl Line {
/// The chat session a turn of this run is sent to — the run the session is
/// on now, which a rollover replaced.
pub(crate) fn chat(&self) -> Option<Arc<ChatSession>> {
    self.run.as_ref().map(Run::chat)
}

/// The run's one mode cell: what a surface states a mode on.
pub(crate) fn mode_cell(&self) -> Option<ModeCell> {
    self.run.as_ref().map(Run::mode_cell)
}

/// The port this run's turns run through, for the operations a surface states
/// without running a turn.
pub(crate) fn run_port(&self) -> Option<Arc<dyn RunPort>> {
    self.run.as_ref().map(Run::run_port)
}

/// The profile this run was configured on and could not open, when there is
/// one: a surface names the absence.
pub(crate) fn skipped_primary(&self) -> Option<&str> {
    self.run.as_ref().and_then(Run::skipped_primary)
}

/// The run's own checkpoints: the rewind an edited resend performs, through
/// exactly the store `/undo` reads.
pub(crate) fn checkpoints(&self) -> Option<crate::chat::RunCheckpoints> {
    self.run
        .as_ref()
        .map(|run| crate::chat::RunCheckpoints::new(Arc::clone(run.job())))
}

/// One operator message, as a turn of the run the session is on now, and the
/// boundary that follows it: the turn's own report is what the surface's front
/// read from it, never a second read of the engine.
pub(crate) async fn turn(&mut self, message: String, front: &crate::front::tui::run_front::Forward) -> Said {
    let message = self.composed(message);
    let Some(session) = self.chat() else {
        return Said::Error(ENDED.to_owned());
    };
    // A message the session holds rather than runs belongs to the turn after it:
    // nothing settled here, so nothing crosses the boundary.
    if !matches!(session.send(message.clone()).await, crate::chat::Admission::Ran) {
        return Said::Nothing;
    }
    let Some(settled) = front.take_settled() else {
        return Said::Nothing;
    };
    self.settled(&message, &settled).await
}

/// Replace the last turn with an edited message: the previous turn's file
/// changes are rewound through the run's own checkpoints — skipping a file
/// another run changed since, and naming it — before the edited line runs.
/// The visible transcript drops the old turn, so the next run replays the
/// edited conversation, not both.
pub(crate) async fn replace_last(
    &mut self,
    message: String,
    front: &crate::front::tui::run_front::Forward,
) -> Said {
    let checkpoints = self.checkpoints();
    Self::rewind_last(
        &mut self.transcript,
        checkpoints.as_ref().map(|run| run as &dyn crate::chat::Checkpoints),
        front,
    )
    .await;
    self.turn(message, front).await
}

/// The rewind half of a replace: drop the replaced turn from the conversation,
/// then put its file changes back through the run's own checkpoints. The
/// confirmation line names what was restored and what another run changed; a
/// refusal is said too, and never stops the edited line from running.
pub(crate) async fn rewind_last(
    transcript: &mut crate::chat::Transcript,
    checkpoints: Option<&dyn crate::chat::Checkpoints>,
    front: &crate::front::tui::run_front::Forward,
) {
    transcript.forget_last();
    let Some(checkpoints) = checkpoints else {
        return;
    };
    // A turn is not running: this operation is queued behind the previous
    // one, and the edited message has not started yet.
    match crate::chat::undo(checkpoints, false).await {
        Ok(words) => front.note(words),
        // Nothing to rewind is still worth stating, and the edited line still
        // runs: the operator asked for it.
        Err(refusal) => front.note(refusal.sentence()),
    }
}

/// The turn a surface's own loop drove, as its front read it: the same
/// boundary a line turn crosses, on the outcome and the words the surface
/// took from its own queue.
pub(crate) async fn settled(
    &mut self,
    prompt: &str,
    settled: &crate::front::SettledTurn,
) -> Said {
    self.settled += 1;
    let output = self.run.as_ref().and_then(Run::take_output);
    self.boundary(&settled.outcome, output, prompt, &settled.record.answer)
        .await
}

/// `/compact`: one compaction now, whether or not a turn crossed the
/// threshold — the operator's own ask, through the same operation.
pub(crate) async fn compact(&mut self) -> Said {
    self.compact_counted().await.0
}

/// The same compaction, with the number of turns it folded: what a surface
/// says the new run carries.
pub(crate) async fn compact_counted(&mut self) -> (Said, usize) {
    let Some(next) = self.next() else {
        return (Said::Error(ENDED.to_owned()), 0);
    };
    let turns = self.transcript.turns().len();
    let bounded = boundary::forced(
        &self.compaction,
        &self.rollover,
        &mut self.run,
        &mut self.transcript,
        || next.compose(),
    )
    .await;
    let summarised = matches!(bounded, Bounded::Rolled { .. });
    let said = self.take(bounded).await;
    let kept = crate::interactive::session::continuation::compaction::manual::COMPACT_KEEP_TAIL_TURNS;
    (said, if summarised { turns.saturating_sub(kept) } else { 0 })
}

/// `/clear`: the latch is re-armed and the conversation rolls away — the
/// chat layer's own clear, then the rollover it asks for.
pub(crate) async fn clear(&mut self) -> Said {
    self.compaction.clear();
    let Some(next) = self.next() else {
        return Said::Nothing;
    };
    let bounded =
        boundary::cleared(&self.rollover, &mut self.run, &mut self.transcript, || {
            next.compose()
        })
        .await;
    self.take(bounded).await
}
}
