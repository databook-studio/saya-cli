// The line loop's own turn, driven through the terminal front (`line.rs`): the
// message a run receives, the stop that ends a turn rather than the run, and the
// boundary that follows it. The full-screen surface drives [`Line::turn`]
// instead, and reads its own queue.

impl Line {
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
}
