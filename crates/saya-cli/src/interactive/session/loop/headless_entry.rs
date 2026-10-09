/// The one per-turn entry every headless path shares: the piped loop
/// pre-trims each line (`handle_line`), while `--turn-file` passes the file
/// bytes verbatim (no trimming, no blank-line skipping). An empty turn here
/// is an errored turn — the stream names it and the caller exits non-zero —
/// never silent success.
fn handle_line_verbatim(
    line: &str,
    ctx: &mut TurnContext,
) -> Result<TurnOutcome, Box<dyn std::error::Error>> {
    let state = &mut *ctx.state;
    let runtime_handle = Arc::clone(&ctx.runtime);
    let runtime: &RuntimeConfig = &runtime_handle;
    let store = ctx.store;
    let state_db = ctx.state_db;
    let format = ctx.format;
    let terminal = ctx.terminal;
    let session = &mut *ctx.session;
    let save_refused = &mut ctx.save_refused;
    if line.trim().is_empty() {
        // Piped input treats blank lines as no-ops. `read_turn_file` rejects
        // an all-blank file before this entry is reached.
        return Ok(TurnOutcome::Completed);
    }
    // A malformed or unknown slash command must not tear down the whole session:
    // surface the parse error (which may carry a "did you mean" hint) and keep looping.
    let parsed = match parse_slash_command(line) {
        Ok(parsed) => parsed,
        Err(error) => {
            session_emit::emit_action(
                SessionAction::Error(error.to_string()),
                format,
                state,
                store,
            )?;
            if let Some(outcome) = persist(state, store, format, save_refused)? {
                return Ok(outcome);
            }
            return Ok(TurnOutcome::Errored);
        }
    };
    // A mode change through `/approvals` carries the activation line with it:
    // under bypass the no-euphemism wording, the staged interpreter facts,
    // and the probe's verdict — said where the mode is set, not just implied
    // by the indicator. The mode before the command decides whether this
    // command newly activated bypass: only then is a consent recorded in
    // the journal; a re-statement over an already-bypass session records
    // none, and a failed journal write is said, not silent.
    let before_mode = state.approval_mode.clone();
    let approvals_set = matches!(parsed, Some(SlashCommand::Approvals(Some(_))));
    let action = match parsed {
        Some(command) => {
            // The answering context before and after the command decides
            // whether this command changed it: a changed key retires the
            // question authority once the command's message is produced, an
            // unchanged key leaves a pending clarification untouched.
            let before = state.context_key();
            let action = state.apply(
                command,
                &runtime
                    .connections
                    .profiles
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            state.abandon_question_when_context_changed(&before, session);
            action
        }
        None => {
            let history = state.provider_request_history();
            let approval = state
                .approval_mode
                .parse()
                .map_err(|error: saya_agent::ApprovalPolicyParseError| error.to_string())?;
            // The turn's decider rides the session's one policy, synced to
            // the current mode (a mid-session `/approval` takes effect next
            // turn, grants carried), so a session grant outlives the turn.
            session.sync_policy(approval);
            // The live task list starts the turn seeded from the record, so
            // the model sees what it last wrote; afterwards the cell is
            // synced back, so `tasks_set` writes persist and ride the next
            // turn.
            session.universe().seed_tasks(state.task_list.clone());
            // The line REPL states the live terminal fact for both
            // capabilities: stdin readability and approval obtainability are
            // the same fact here — no modal, no alternate screen. The TUI
            // states them separately (`tui::agent::approval_capabilities`).
            let scope = session.begin_question(runtime)?;
            // The settle must know WHICH question completed: the scope this
            // begin handed out, cloned before the request consumes it.
            let completed = scope.clone();
            let request_result = block_on(super::super::session_request::run(
                runtime,
                line,
                approval,
                session.policy(),
                Some(session.journal()),
                terminal,
                terminal,
                state.prompt_overrides(),
                history,
                format,
                state_db,
                session.universe(),
                state.agent_mode_parsed(),
                state.prior_tool_outcomes.clone(),
                state.continuation.clone(),
                state.compaction_summary_text(),
                scope,
            ));
            // The tool executor writes the session's live task cell before
            // a later provider request can fail. Once the request returns,
            // persist that accepted posture at the settlement boundary for
            // every terminal outcome, including cancellation and error.
            state.task_list = session.universe().tasks().current();
            match request_result {
                Ok(settled) => match settled.result {
                    PromptResult::Completed(output) => {
                        let clarification = output.events.iter().any(|event| {
                            matches!(event, saya_agent::AgentEvent::ClarificationNeeded { .. })
                        });
                        session.settle_question(&completed, clarification);
                        state.record_agent_output(line, &output);
                        // Feed the session accumulator so /usage is honest in
                        // headless mode too (the TUI does this in drain_stream).
                        state.usage.record(&output.usage);
                        // Fold the extraction call's usage into the learning total
                        // before the output moves into the action. `None` (no
                        // extraction or no response) records nothing, so a session
                        // with learning disabled is unaffected.
                        state.usage.record_learning(output.learning_usage);
                        // The shared automatic-compaction decision at the same settlement
                        // the TUI reads; a pending clarification and a stop refuse
                        // inside the adapter.
                        if let Some(compact) =
                            super::continuation::compaction::headless::settle_auto_compact(
                                runtime,
                                state,
                                session,
                                &output,
                                clarification,
                                false,
                            )
                        {
                            session_emit::emit_action(compact, format, state, store)?;
                        }
                        ctx.last_turn = Some(TurnSummary {
                            // A completed turn reports completed, budget latched
                            // or not (the partial wording is said separately).
                            status: prose_status(ProseOutcome::Completed, false),
                            tally: settled.tally,
                        });
                        SessionAction::Agent(*output)
                    }
                    PromptResult::Cancelled {
                        answering_usage,
                        learning_usage,
                    } => {
                        session.abandon_question();
                        if let Some(usage) = answering_usage {
                            state.usage.answering.record_call(&usage);
                        }
                        state.usage.record_learning(learning_usage);
                        let budget = budget_dimension(completed.cancellation().reason());
                        ctx.last_turn = Some(TurnSummary {
                            status: prose_status(ProseOutcome::Cancelled, budget.is_some()),
                            tally: settled.tally,
                        });
                        match budget {
                            Some(dimension) => {
                                SessionAction::Error(budget_partial_message(dimension))
                            }
                            None => SessionAction::Cancelled,
                        }
                    }
                    PromptResult::Failed(error, usage) => {
                        session.abandon_question();
                        if let Some(usage) = usage {
                            state.usage.answering.record_call(&usage);
                        }
                        let budget = budget_dimension(completed.cancellation().reason());
                        ctx.last_turn = Some(TurnSummary {
                            status: prose_status(ProseOutcome::Failed, budget.is_some()),
                            tally: settled.tally,
                        });
                        match budget {
                            Some(dimension) => {
                                SessionAction::Error(budget_partial_message(dimension))
                            }
                            None => SessionAction::Error(error.to_string()),
                        }
                    }
                },
                Err(error) => {
                    session.abandon_question();
                    SessionAction::Error(error.to_string())
                }
            }
        }
    };
    if matches!(action, SessionAction::Exit) {
        return Ok(TurnOutcome::Exit);
    }
    finish_action(action, before_mode, approvals_set, ctx)
}
