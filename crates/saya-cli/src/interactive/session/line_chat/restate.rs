// The run a state change gives up (`line.rs`): a command that changes the
// model, the provider, the connection or the privacy setting states the facts
// the next run composes over, and the next request of the same conversation is
// composed from them — before the first turn by opening the run again, after a
// turn by rolling the ended run into a successor seeded with the transcript.

impl Line {
    /// A command changed the facts a run is composed from — the model, the
    /// provider, the connection, the privacy setting.
    ///
    /// The next request of this conversation is composed from `overrides`, never
    /// from what the run was opened with: the session's runtime states them, and
    /// the run is replaced before the next line.
    ///
    /// - **Before the first turn** the run holds no conversation to keep: it is
    ///   given up the way the line front gives up its prepared one — the session's
    ///   own quit and the job's end, which frees the folder and leaves no record of
    ///   a run that took no turn — and the next message composes the run it asks
    ///   for.
    /// - **After a turn** the conversation is kept: the run that ended is rolled
    ///   into a new job seeded with the transcript, exactly as a compaction rolls,
    ///   so the next request carries every settled turn and the state the command
    ///   stated.
    ///
    /// A command that states what the session already holds says nothing and moves
    /// nothing. A change that cannot be prepared leaves the run the session is on
    /// exactly as it was, and says why.
    pub(crate) async fn restate(&mut self, overrides: PromptOverrides) -> Said {
        if self.parts.overrides == overrides {
            return Said::Nothing;
        }
        let cause = Self::cause_of(&self.parts.overrides, &overrides);
        // The parts the next run composes over, stated before anything is given up:
        // the run a turn of this surface runs is composed over the session's own
        // runtime, so the change is stated there too, through the one function
        // every turn's inputs are resolved through.
        let mut parts = self.parts.clone();
        let mut config = (*parts.runtime).clone();
        config.resolved.ai = crate::agent::turn_config::effective_ai(&config.resolved.ai, &overrides);
        parts.runtime = Arc::new(config);
        parts.overrides = overrides;
        if self.settled == 0 {
            self.reopen(parts).await
        } else {
            self.reroll(parts, cause).await
        }
    }

    /// The run before the first turn, given up and composed again from `parts`: the
    /// session's own quit and the job's end, which frees the folder and leaves no
    /// record of a run that took no turn, and then the next run — composed over the
    /// host the updated parts state, exactly as the launch composes its own.
    async fn reopen(&mut self, parts: LineSession) -> Said {
        // The host the next run composes over is stated from the parts as they are
        // now — the session's own runtime and folder — before the run that ended is
        // given up, so a host that cannot be opened leaves the session's run alone.
        let hosted = match super::open::host(&parts).await {
            Ok(hosted) => hosted,
            Err(reason) => return Said::Error(Self::not_prepared(&reason)),
        };
        let Some(run) = self.run.take() else {
            return Said::Error(ENDED.to_owned());
        };
        run.end_prepared_run().await;
        drop(run);
        let front = Arc::clone(&self.front);
        match open_run(&parts, hosted, front, self.metrics.clone()).await {
            Ok((run, application, shape)) => {
                self.parts = parts;
                self.shape = shape;
                self.run = Some(run);
                self.rollover = Rollover::new(application);
                Said::Nothing
            }
            Err(reason) => Said::Error(Self::not_prepared(&reason)),
        }
    }

    /// The run after a turn, rolled into the next job seeded with the conversation:
    /// the operation a compaction rolls, with the cause the change stated. The
    /// successor composes over the host the updated parts state — the model, the
    /// provider and the connection the command named.
    async fn reroll(&mut self, parts: LineSession, cause: crate::chat::RolloverCause) -> Said {
        // The host and the successor's stated facts are built before the run that
        // ended is handed over, so a failure leaves it exactly as it was.
        let hosted = match super::open::host(&parts).await {
            Ok(hosted) => hosted,
            Err(reason) => return Said::Error(Self::not_prepared(&reason)),
        };
        if self.run.is_none() {
            return Said::Error(ENDED.to_owned());
        }
        let next = Next::new(
            self.next_name(),
            Arc::clone(&self.front) as Arc<dyn ChatFront>,
            parts.clone(),
            hosted.shape.clone(),
        );
        let composed = match next.compose().await {
            Ok(composed) => composed,
            Err(reason) => return Said::Error(Self::not_prepared(&reason)),
        };
        let application = hosted.application;
        let rollover = Rollover::new(Arc::clone(&application));
        let ending = super::run::hand_over(self.run.take());
        match rollover
            .roll(ending, cause, composed, &mut self.transcript)
            .await
        {
            Ok(successor) => {
                self.parts = parts;
                self.shape = hosted.shape;
                self.rollover = rollover;
                self.replaced(successor).await;
                Said::Nothing
            }
            Err(error) => {
                self.run = None;
                Said::Error(Self::not_prepared(&error.to_string()))
            }
        }
    }

    /// Why the run after a state change never opened, in words a person reads.
    fn not_prepared(reason: &str) -> String {
        format!("the run after the change could not be prepared: {reason}")
    }

    /// What a state change changed, as the rollover names it.
    fn cause_of(before: &PromptOverrides, after: &PromptOverrides) -> crate::chat::RolloverCause {
        if before.provider != after.provider
            || before.endpoint != after.endpoint
            || before.endpoint_bound != after.endpoint_bound
        {
            crate::chat::RolloverCause::Provider
        } else if before.model != after.model {
            crate::chat::RolloverCause::Model
        } else if before.allow_data_sharing != after.allow_data_sharing {
            crate::chat::RolloverCause::Privacy
        } else {
            crate::chat::RolloverCause::Connection
        }
    }
}
