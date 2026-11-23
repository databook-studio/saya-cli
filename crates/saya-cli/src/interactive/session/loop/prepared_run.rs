// The run composed while the operator is still at the first prompt.
//
// A session's first plain line is a turn of a run, and composing that run —
// the folder's job, its lease, its provider, its prompt — costs the operator a
// wait they need not have: the time between the prompt and the first keystroke
// is theirs, and the run can be composed in it. The loop therefore starts the
// composition before it reads anything (`LineChat::prepare`) and the first line
// adopts it (`Preparing::adopt`): send only ever starts the turn.
//
// **One run, one statement of its facts.** `action_message` states what a first
// run is composed from, and this file starts the same composition over the same
// statement — a session that adopts it and one that composes on its first line
// open the same run.
//
// **Nothing is left working.** A prepared run the first line does not adopt —
// a slash command changed what a run is composed from, a refusal, or a session
// that ends without sending — is given up here: its run is closed where one
// opened, so a session that says nothing leaves no run working.

impl TurnContext<'_> {
    /// Starts composing the session's first run, before the first line is read.
    ///
    /// A session whose first run cannot even be stated — no folder, a mode the
    /// record holds badly — starts nothing here: the line that asks for the run
    /// states the reason in the same words it always did.
    fn prepare_first_run(&mut self) {
        // One prepare per session: the run is composed before the first line,
        // and every later line is a turn of it.
        if self.chat.is_some() || self.prepared.is_some() {
            return;
        }
        let tasks = self.session.universe().tasks();
        let parts = first_message_parts(
            &*self.state,
            &*self.session,
            Arc::clone(&self.runtime),
            self.state_db,
            self.format,
            self.terminal,
            tasks,
        );
        let record = surface_record(&*self.state);
        if let Ok(parts) = parts {
            self.prepared = Some(LineChat::prepare(parts, record));
        }
    }

    /// Gives up the prepared run the loop is holding, if it holds one: the line
    /// that made it stale, or the loop's own end, states what follows.
    fn give_up_prepared_run(&mut self) {
        if let Some(prepared) = self.prepared.take() {
            prepared.abandon();
        }
    }
}
