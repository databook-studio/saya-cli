impl SessionState {
    pub fn apply(&mut self, command: SlashCommand, available: &[String]) -> SessionAction {
        match command {
            SlashCommand::Connect(name) => {
                if !available.iter().any(|profile| profile == &name) {
                    return SessionAction::Error(format!("Unknown configured profile: {name}"));
                }
                self.profile = Some(name.clone());
                SessionAction::Message(format!("Selected profile: {name}"))
            }
            SlashCommand::Connections => SessionAction::Message(if available.is_empty() {
                "No configured connection profiles. Add one to .saya/connections.toml \
                 (see `saya config init`) or pass --connections."
                    .into()
            } else {
                format!("Profiles: {}", available.join(", "))
            }),
            SlashCommand::Include(name) => {
                if !available.iter().any(|profile| profile == &name) {
                    return SessionAction::Error(format!("Unknown configured profile: {name}"));
                }
                if !self.included_profiles.contains(&name) {
                    self.included_profiles.push(name.clone());
                }
                SessionAction::Message(format!("Included profile: {name}"))
            }
            SlashCommand::Exclude(name) => {
                self.included_profiles.retain(|item| item != &name);
                SessionAction::Message(format!("Excluded profile: {name}"))
            }
            SlashCommand::Provider(value) => {
                if let Some(value) = value {
                    if saya_config::AiProvider::parse(&value).is_none() {
                        return SessionAction::Error(format!(
                            "Unsupported provider: {value}. Use ollama, openai, openai_compatible, anthropic, or gemini."
                        ));
                    }
                    if self.provider != value {
                        self.provider_endpoint = None;
                        self.provider_endpoint_bound = true;
                    }
                    self.provider = value;
                    SessionAction::Message(format!("Provider: {}", self.provider))
                } else {
                    SessionAction::Message(format!(
                        "Provider: {} (available: {})",
                        self.provider,
                        available_providers().join(", ")
                    ))
                }
            }
            SlashCommand::Model(value) => {
                if let Some(value) = value {
                    self.model = value;
                    SessionAction::Message(format!("Model: {}", self.model))
                } else {
                    let models = known_models(&self.provider);
                    if models.is_empty() {
                        SessionAction::Message(format!(
                            "Model: {} (no suggestions for provider {})",
                            self.model, self.provider
                        ))
                    } else {
                        SessionAction::Message(format!(
                            "Model: {}\nKnown models for {}: {}",
                            self.model,
                            self.provider,
                            models.join(", ")
                        ))
                    }
                }
            }
            SlashCommand::Privacy(value) => {
                if let Some(value) = value {
                    self.allow_data_sharing = value;
                }
                // Mirrors the status bar's `sharing:` segment so the two
                // surfaces cannot tell a reader opposite things about the
                // same state: `sharing:on` means row values are sent to the
                // provider, `sharing:off` means they are not.
                SessionAction::Message(format!(
                    "Cloud data sharing: {}",
                    if self.allow_data_sharing { "on" } else { "off" }
                ))
            }
            SlashCommand::Approvals(value) => {
                if let Some(value) = value {
                    self.approval_mode = approval_name(value);
                }
                SessionAction::Message(format!("Approval mode: {}", self.approval_mode))
            }
            SlashCommand::Mode(value) => {
                if let Some(value) = value {
                    self.agent_mode = value.as_str().into();
                }
                SessionAction::Message(mode_message(&self.agent_mode))
            }
            SlashCommand::Schema(refresh) => SessionAction::Schema(refresh),
            SlashCommand::Sql(query) => SessionAction::Sql(query),
            SlashCommand::Export(request) => SessionAction::Export(request),
            SlashCommand::Report(request) => SessionAction::Report(request),
            SlashCommand::Chart(args) => SessionAction::Chart(args),
            SlashCommand::Explain(sql) => SessionAction::Explain(sql),
            SlashCommand::Compact => SessionAction::Compact,
            SlashCommand::Clear => {
                self.messages.clear();
                self.turns.clear();
                self.task_list = Default::default();
                self.compaction_summary = None;
                self.compacted_turns = 0;
                self.observations = Default::default();
                self.usage = Default::default();
                self.context_warned = false;
                self.auto_compact_failed = false;
                // A typed continuation described the cleared conversation: it
                // goes with it. An `Unsupported` one is a newer binary's data
                // and survives; the parent link is lineage, not conversation,
                // and survives too.
                if matches!(
                    self.continuation,
                    Some(saya_store::StoredContinuation::Current(_))
                ) {
                    self.continuation = None;
                }
                // The transcript keeps what was said; the model's working
                // memory does not. Say so, since there is no undo.
                SessionAction::Message(
                    "Conversation context cleared — the model will not remember earlier turns.                      This cannot be undone; use /export first if you need a copy."
                        .into(),
                )
            }
            SlashCommand::History => SessionAction::History,
            SlashCommand::Doctor => SessionAction::Doctor,
            SlashCommand::Usage => SessionAction::Message(self.usage.render()),
            SlashCommand::Workspace => SessionAction::Message(match self.workspace_root.as_deref() {
                Some(root) => format!("workspace: {root}"),
                None => "no workspace is bound: writes, downloads, and run_program are                          unavailable — launch inside a git worktree or with --workspace <dir>"
                    .to_string(),
            }),
            SlashCommand::Thinking(value) => {
                if let Some(value) = value {
                    self.show_thinking = value;
                } else {
                    self.show_thinking = !self.show_thinking;
                }
                // The toggle applies to subsequent turns only: reasoning from
                // earlier turns was not retained (it lives on the per-call
                // ChatResponse and the in-memory ReasoningText event, neither
                // stored on the session), so there is nothing to re-render.
                SessionAction::Message(format!(
                    "Thinking display: {}",
                    if self.show_thinking { "on" } else { "off" }
                ))
            }
            SlashCommand::Sessions => SessionAction::History,
            SlashCommand::Resume(id) => SessionAction::Resume(id),
            SlashCommand::Rotate => SessionAction::Rotate,
            SlashCommand::Tasks(subcommand) => match subcommand.as_deref() {
                // `/tasks clear` empties the stored list, so the next turn
                // injects no task block. Nothing else edits from here: the
                // model owns the content, the user owns whether it exists.
                Some("clear") => {
                    self.task_list = Default::default();
                    SessionAction::Message("Task list cleared.".into())
                }
                _ => SessionAction::Message(crate::interactive::session::tasks::view::render_tasks_view(
                    &self.task_list,
                )),
            },
            SlashCommand::Notes(command) => {
                // `/notes …` — the user's own notes on the session's typed
                // continuation record, through the one shared `run_notes`
                // operation both surfaces run. The `/tasks clear` shape:
                // `apply` owns the state, so the change happens here and the
                // loops' normal Message/Error paths emit it and save the
                // session on their normal paths. No loop intercept is owed.
                let (ok, message) = crate::interactive::session::lifecycle::notes::run_notes(self, command);
                if ok {
                    SessionAction::Message(message)
                } else {
                    SessionAction::Error(message)
                }
            }
            SlashCommand::Columns(_) => {
                SessionAction::Message("Column selection applies in the interactive TUI.".into())
            }
            SlashCommand::Help(topic) => {
                SessionAction::Message(crate::slash::help_for(topic.as_deref()))
            }
            SlashCommand::Contracts(command) => SessionAction::Contracts(command),
            SlashCommand::Investigation(command) => SessionAction::Investigation(command),
            SlashCommand::Run(args) => {
                // Plan's escape hatch stays shut here: a run writes through
                // its own declared capabilities rather than the session's
                // mode, so a Plan session that spawned one would step around
                // the posture it just set. The refusal names both facts — the
                // session is in Plan mode, and the run would carry its own
                // capabilities — and the remedy (`/mode build`), in the voice
                // of the run-boundary precedent (`RunMode::admit`): never a
                // silently narrowed run, never a silent allow. Cancelling a
                // run and listing runs are read-shaped and pass through.
                if self.agent_mode_parsed() == saya_agent::AgentMode::Plan {
                    return SessionAction::Error(
                        "this session is in Plan mode, and a run writes through its \
                         own declared capabilities rather than the session's mode — \
                         switch to Build with `/mode build` if the run is intended"
                            .into(),
                    );
                }
                SessionAction::Run(args)
            }
            SlashCommand::RunCancel(run_id) => SessionAction::RunCancel(run_id),
            SlashCommand::Runs(run_id) => SessionAction::Runs(run_id),
            SlashCommand::Allow(tokens) => SessionAction::Allow(tokens),
            SlashCommand::Grants => SessionAction::Grants,
            SlashCommand::Context => SessionAction::Context,
            SlashCommand::SessionsList => SessionAction::SessionsList,
            SlashCommand::SessionsDelete(id) => SessionAction::SessionsDelete(id),
            SlashCommand::Preferences(command) => SessionAction::Preferences(command),
            SlashCommand::Recovery(command) => SessionAction::Recovery(command),
            SlashCommand::Setup => SessionAction::Setup,
            SlashCommand::Reload => SessionAction::Reload,
            SlashCommand::Exit => SessionAction::Exit,
        }
    }

    pub(crate) fn prompt_overrides(&self) -> PromptOverrides {
        PromptOverrides {
            provider: saya_config::AiProvider::parse(&self.provider),
            model: Some(self.model.clone()),
            allow_data_sharing: Some(self.allow_data_sharing),
            profile: self.profile.clone(),
            included_profiles: self.included_profiles.clone(),
            endpoint: self.provider_endpoint.clone(),
            endpoint_bound: self.provider_endpoint_bound,
        }
    }
}
