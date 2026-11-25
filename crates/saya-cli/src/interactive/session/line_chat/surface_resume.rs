// The session a `/resume` opens in place of this one (`line.rs`): the run this
// session is on is given up, and the record is opened the one way a launch
// opens it, so the next message is a turn of the run that record names.

impl Line {
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
    fn workspace_of(
        folder: Option<&std::path::Path>,
    ) -> Option<Arc<saya_harness::workspace::Workspace>> {
        let folder = folder?;
        saya_harness::workspace::Workspace::open(folder)
            .ok()
            .map(Arc::new)
    }
}
