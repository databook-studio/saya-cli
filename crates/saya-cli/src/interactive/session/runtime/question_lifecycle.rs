impl SessionRuntime {
    pub(crate) fn begin_question(
        &mut self,
        runtime: &crate::config::runtime::RuntimeConfig,
    ) -> Result<QuestionScope, String> {
        let scope = match std::mem::replace(&mut self.question, QuestionLifecycle::Idle) {
            QuestionLifecycle::Idle => {
                QuestionScope::new(runtime, saya_agent::CancellationToken::new())
            }
            QuestionLifecycle::PendingClarification(scope) => scope,
            QuestionLifecycle::Active(scope) => {
                self.question = QuestionLifecycle::Active(scope);
                return Err("an answering question is already active".into());
            }
        };
        self.question = QuestionLifecycle::Active(scope.clone());
        Ok(scope)
    }

    /// Settles the question that actually completed. Only the scope
    /// `begin_question` handed out may settle itself: a completion of any
    /// other scope — and any completion while idle or pending clarification
    /// — is ignored outright, so a late completion of a stale question can
    /// never restore or change the live question's state.
    pub(crate) fn settle_question(&mut self, completed: &QuestionScope, clarification: bool) {
        let settling = match &self.question {
            QuestionLifecycle::Active(scope) if scope.is_same_question(completed) => scope.clone(),
            _ => return,
        };
        if clarification && !settling.cancellation().is_cancelled() {
            self.question = QuestionLifecycle::PendingClarification(settling);
        } else {
            settling.finish_segment();
            self.question = QuestionLifecycle::Idle;
        }
    }

    pub(crate) fn abandon_question(&mut self) {
        let previous = std::mem::replace(&mut self.question, QuestionLifecycle::Idle);
        match previous {
            QuestionLifecycle::Active(scope) | QuestionLifecycle::PendingClarification(scope) => {
                scope.cancellation().cancel();
                scope.finish_segment();
            }
            QuestionLifecycle::Idle => {}
        }
    }

    /// Keeps the just-settled question's metrics snapshot; only the last is held.
    pub(crate) fn remember_question_metrics(&mut self, snapshot: LocalPhaseSnapshot) {
        self.last_question_metrics = Some(snapshot);
    }

    /// Takes the kept snapshot once, for the print-after-restore site.
    pub(crate) fn take_question_metrics(&mut self) -> Option<LocalPhaseSnapshot> {
        self.last_question_metrics.take()
    }

    /// The session's local metrics collector, when local metrics are on:
    /// the handle the headless compaction sites record and print through.
    /// Runtime-only — never serialized, never in `SessionState`.
    pub(crate) fn session_metrics(&self) -> Option<Arc<LocalMetrics>> {
        self.session_metrics.clone()
    }
}
