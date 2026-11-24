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

/// The session a `/resume` opens in place of this one: the run it is on is given
/// up — the chat session's close and the job's end, so the folder's lease is
/// free — and the record is opened the one way a launch opens it
/// ([`super::open::open_surface`]), so the next message is a turn of the run that
/// record names. The operation the record itself states (its link's decision, its
/// read-only sentence) is read there, never here.
///
/// The record was loaded, saved and checked by the shared operation the line
/// front's `/resume` also goes through
/// ([`crate::interactive::session::lifecycle::resume_mid`]).
///
/// The answer is the line the surface drives from here on, and either the
/// sentence the record stated about itself or the refusal. A run that could not
/// be opened keeps the line — with the run it already gave up — rather than
/// losing the session: the operator reads why and still has a surface.
pub(crate) async fn resumed(
    mut self,
    loaded: &crate::interactive::session::state::SessionState,
) -> (Line, Result<Option<String>, String>) {
    let record = crate::interactive::session::lifecycle::resume_mid::record_of(loaded);
    let parts = self.resumed_parts(loaded);
    let front = Arc::clone(&self.front);
    self.give_up().await;
    match super::open::open_surface(parts, &record, front).await {
        Ok(opened) => {
            let (line, said) = opened.into_parts();
            (line, Ok(said))
        }
        Err(sentence) => (self, Err(sentence)),
    }
}

/// Gives this session's run up: the chat session's own close and the job's end,
/// so the folder's lease is free for the run that follows — the same give-up a
/// slash command states before its first turn, and the one a resume states for the
/// session it leaves.
async fn give_up(&mut self) {
    if let Some(run) = self.run.take() {
        run.end_prepared_run().await;
        drop(run);
    }
}

/// The parts a resumed record's run is composed from: the same launch — its
/// configuration, its store, its surface and its state root — over the session
/// the record states, with the conversation it holds as the run's seed.
fn resumed_parts(&self, loaded: &crate::interactive::session::state::SessionState) -> LineSession {
    let mut parts = self.parts.clone();
    parts.session = loaded.id.clone();
    parts.folder = loaded.workspace_root.as_deref().map(std::path::PathBuf::from);
    parts.mode = loaded
        .approval_mode
        .parse()
        .unwrap_or(saya_agent::ApprovalPolicy::Ask);
    parts.overrides = loaded.prompt_overrides();
    parts.seed = loaded.replayed_pairs();
    // A resumed session inherits no grant: the grants are this process's facts
    // about the session that stated them, and the record states its own turn.
    parts.grants = saya_agent::SessionGrants::default();
    parts.facts = Some(crate::chat::SessionFacts::of(
        loaded,
        Self::workspace_of(parts.folder.as_deref()),
    ));
    // The run's one task cell: the record's own list, so the `checklist` a turn
    // writes is the resumed session's.
    if let Some(tasks) = parts.tasks.as_ref() {
        tasks.replace(loaded.task_list.clone());
    }
    parts
}

/// The folder a resumed session's facts are rendered over, opened the one way the
/// file tools open one.
fn workspace_of(folder: Option<&std::path::Path>) -> Option<Arc<saya_harness::workspace::Workspace>> {
    let folder = folder?;
    saya_harness::workspace::Workspace::open(folder)
        .ok()
        .map(Arc::new)
}

/// Sends one operator line as a turn of the run, and crosses the boundary
/// with the turn's own report: the compaction reads the output the engine
/// stated for this turn, never an earlier one.
pub(super) async fn send(&mut self, message: String) -> Result<Option<Sent>, String> {
    let message = self.composed(message);
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

/// The operator's message as the run receives it: `@path` mentions carry the
/// files they name, read the way the file tools read them.
///
/// One implementation for both fronts, because both send through this line: the
/// full-screen surface drives [`Line::turn`] and the line loop drives
/// [`Line::send`], and neither composes a message of its own. A run with no
/// workspace attaches nothing — it has no file a run is allowed to read — and a
/// message with no mention is returned exactly as it was typed, so a request
/// that points at nothing costs what it always did.
fn composed(&self, message: String) -> String {
    if crate::chat::mentions::mentioned(&message).is_empty()
        || self.shape.workspace() == crate::chat::Workspace::None
    {
        return message;
    }
    let Some(folder) = self.parts.folder.as_deref() else {
        return message;
    };
    // The run's own contained workspace, opened the one way the file tools open
    // it, so the mention's read is the read tool's read.
    match saya_harness::workspace::Workspace::open(folder) {
        Ok(workspace) => crate::chat::mentions::attach(&message, Some(&workspace)),
        Err(_) => message,
    }
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
