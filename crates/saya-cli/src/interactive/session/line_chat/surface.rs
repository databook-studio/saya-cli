// The surface's half of one line session: the run it is on now, the boundary a
// turn of it crosses when the surface's own loop drove that turn, and the run a
// state change gives up (`line.rs`).
//
// The full-screen surface holds the same `Line` the line loop drives — the
// transcript, the compaction and the rollover — and drives its own turn through
// it, so `/clear`, `/compact` and `/rotate` perform the line front's own
// operations on the live run rather than on the run that follows. What differs
// is only who reads the settled turn: the line loop reads its terminal front
// ([`Line::send`]), and this surface reads its own queue ([`Line::settled`]).

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

/// One operator message, as a turn of the run the session is on now, and the
/// boundary that follows it: the turn's own report is what the surface's front
/// read from it, never a second read of the engine.
pub(crate) async fn turn(&mut self, message: String, front: &crate::front::tui::run_front::Forward) -> Said {
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

/// A command changed the facts a run is composed from — the model, the
/// provider, the connection, the privacy setting.
///
/// The line front gives up the run it composed ahead of its first line when
/// a slash command changes those facts, and that line composes the run it
/// asks for (`loop/headless_entry`). This surface's launch run is the same
/// run, so it is given up the same way — the session's own quit and the
/// job's end, which frees the folder and leaves no record of a run that took
/// no turn — and the next message is composed from `overrides`, the session
/// as the command stated it. A run that has already taken a turn keeps what
/// it was opened with, exactly as an open run does on the line front.
pub(crate) async fn restate(&mut self, overrides: PromptOverrides) -> Said {
    if self.settled > 0 {
        return Said::Nothing;
    }
    let Some(run) = self.run.take() else {
        return Said::Error(ENDED.to_owned());
    };
    // The run is given up the way the line front gives up its prepared one: the
    // session's own quit and the job's end, which frees the folder and leaves no
    // record of a run that took no turn. Only then is the next one composed.
    run.end_prepared_run().await;
    drop(run);
    // The run a turn of this surface runs is composed over the session's own
    // runtime, so the change is stated there too: the model, the provider and
    // the privacy setting are resolved through the one function every turn's
    // inputs are resolved through, never restated here.
    let mut config = (*self.parts.runtime).clone();
    config.resolved.ai = crate::agent::turn_config::effective_ai(&config.resolved.ai, &overrides);
    self.parts.runtime = Arc::new(config);
    self.parts.overrides = overrides;
    // The host the next run composes over is stated from the parts as they are
    // now — the session's own runtime and folder — exactly as the launch states
    // its own, and the session's rollover moves to it, so every run this session
    // prepares after the change composes over the same one.
    let hosted = match super::open::host(&self.parts).await {
        Ok(hosted) => hosted,
        Err(reason) => {
            return Said::Error(format!(
                "the run after the change could not be prepared: {reason}"
            ));
        }
    };
    let front = Arc::clone(&self.front);
    match open_run(&self.parts, hosted, front, self.metrics.clone()).await {
        Ok((run, application, shape)) => {
            self.shape = shape;
            self.run = Some(run);
            self.rollover = Rollover::new(application);
            Said::Nothing
        }
        Err(reason) => Said::Error(format!(
            "the run after the change could not be prepared: {reason}"
        )),
    }
}

/// Sends one operator line as a turn of the run, and crosses the boundary
/// with the turn's own report: the compaction reads the output the engine
/// stated for this turn, never an earlier one.
pub(super) async fn send(&mut self, message: String) -> Result<Option<Sent>, String> {
    let Some(run) = self.run.as_ref() else {
        return Err(ENDED.to_owned());
    };
    // A surface that reads its own turn states it (`Line::settled`): this
    // path is the terminal front's, and needs the front it reads.
    let Some(front) = self.reads.as_ref().map(Arc::clone) else {
        return Err(SURFACE_DRIVES.to_owned());
    };
    // This turn's own stop token, held here as well as by the front: a bare
    // Esc at an approval prompt cancels it from the terminal owner, and the
    // turn — never the run — is what ends when it does.
    let turn = CancellationToken::new();
    front.set_turn(turn.clone());
    // A one-turn surface's question carries its own stop — the elapsed
    // budget its scope states — and that stop ends the turn: there is no
    // next line for it to leave open.
    let question = front.one_turn_stop();
    let question_stop = async move {
        match question {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    {
        let send = run.session().send(message.clone());
        tokio::pin!(send);
        tokio::select! {
            _ = &mut send => {}
            _ = tokio::signal::ctrl_c() => {
                // The turn's own stop and the single instant the
                // settlement sample is measured from. The message does not
                // run, and the run stays open for the next line.
                front.stop_turn();
                run.session().cancel().await;
                send.await;
            }
            _ = turn.cancelled() => {
                // The terminal owner's own stop, recorded at the one stop
                // instant: the question the turn left pending ends with it,
                // and the run stays open for the next line.
                front.stop_turn();
                run.session().cancel().await;
                send.await;
            }
            _ = question_stop => {
                // The question's configured budget ran out. Its stop is
                // already latched, so the settlement keeps that class; the
                // turn ends here rather than starting another round.
                front.stop_turn();
                run.session().cancel().await;
                send.await;
            }
        }
    }
    let Some(outcome) = front.take_outcome() else {
        return Ok(None);
    };
    let record = front.take_record();
    let output = self.run.as_ref().and_then(Run::take_output);
    let said = self
        .boundary(&outcome, output.clone(), &message, &record.answer)
        .await;
    self.settled += 1;
    Ok(Some(Sent {
        turn: crate::front::SettledTurn { outcome, record },
        output,
        said,
    }))
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
