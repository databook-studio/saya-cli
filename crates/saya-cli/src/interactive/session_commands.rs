use super::session_state::SessionState;
use crate::agent::runtime::PromptOverrides;
use crate::cli::ContractsCommand;
use crate::slash::SlashCommand;
use saya_agent::AgentOutput;
use saya_agent::ApprovalPolicy;

// `Agent(AgentOutput)` carries agent events backed by `serde_json::Value`, so
// this enum is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionAction {
    Message(String),
    /// `/compact` — shrink working memory through a bounded summariser call.
    /// The loops intercept this (it needs the provider runtime); the arm here
    /// keeps the match exhaustive.
    Compact,
    Agent(AgentOutput),
    Cancelled,
    NotImplemented(String),
    Error(String),
    History,
    /// Run `config doctor` and surface its report as a message.
    Doctor,
    Resume(String),
    Schema(bool),
    Sql(String),
    Export(String),
    Chart(String),
    Explain(String),
    /// A contract slash command, translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces. The loops hand it to the
    /// shared `run_contracts` dispatcher — no second parsing or DTO mapping.
    Contracts(ContractsCommand),
    /// `/run <tail>` — spawn the nested `saya run` child with the raw tail
    /// verbatim (`interactive::session_run` owns the spawn and the
    /// stream-passthrough rule). Carrying the tail, not parsed parts, is the
    /// point: the child's own CLI parser stays the authority.
    Run(String),
    /// `/run cancel <id>` — record a run cancelled through the shared
    /// `run_management` dispatcher, the path `saya run cancel` uses.
    RunCancel(String),
    /// `/runs [id]` — list runs, or show one, through the shared
    /// `run_management` dispatcher, so the slash rendering is the headless
    /// rendering byte for byte.
    Runs(Option<String>),
    /// `/allow <scopes…>` — the tokens as stated. The loops (headless and
    /// TUI) hand them to the shared `session_grants::allow`, which parses
    /// them on the session surface and seeds the session's one grant store;
    /// the loops own the policy, so the seeding lives there.
    Allow(Vec<String>),
    /// `/grants` — the session grant store listed verbatim by the shared
    /// `session_grants::listing`.
    Grants,
    Exit,
}

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
            SlashCommand::Export(path) => SessionAction::Export(path),
            SlashCommand::Chart(args) => SessionAction::Chart(args),
            SlashCommand::Explain(sql) => SessionAction::Explain(sql),
            SlashCommand::Compact => SessionAction::Compact,
            SlashCommand::Clear => {
                self.messages.clear();
                self.turns.clear();
                self.task_list = Default::default();
                self.compaction_summary = None;
                self.compacted_turns = 0;
                self.usage = Default::default();
                self.context_warned = false;
                self.auto_compact_failed = false;
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
            SlashCommand::Columns(_) => {
                SessionAction::Message("Column selection applies in the interactive TUI.".into())
            }
            SlashCommand::Help(topic) => {
                SessionAction::Message(crate::slash::help_for(topic.as_deref()))
            }
            SlashCommand::Contracts(command) => SessionAction::Contracts(command),
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
        }
    }
}

fn approval_name(policy: ApprovalPolicy) -> String {
    match policy {
        ApprovalPolicy::Ask => "ask",
        ApprovalPolicy::ReadOnly => "read-only",
        ApprovalPolicy::Never => "never",
        ApprovalPolicy::Bypass => "bypass",
    }
    .into()
}

/// The `/mode` answer: the session's current posture, with what it means.
/// Set when the command named a mode, reported either way — the `Approvals`
/// arm's shape, so the two sibling commands read alike. Both answers carry
/// the shared bypass-composition sentence (see
/// [`crate::slash::PLAN_BYPASS_SENTENCE`]) in the same words as the
/// detailed help entry.
fn mode_message(mode: &str) -> String {
    match mode {
        "plan" => format!(
            "Mode: plan — read-only; write-shaped tools are hidden and refuse. {}",
            crate::slash::PLAN_BYPASS_SENTENCE
        ),
        _ => format!(
            "Mode: build — writes allowed, asks per approval policy. {}",
            crate::slash::PLAN_BYPASS_SENTENCE
        ),
    }
}

fn available_providers() -> &'static [&'static str] {
    &[
        "ollama",
        "openai",
        "openai_compatible",
        "anthropic",
        "gemini",
    ]
}

/// Curated suggestions for models per provider (convenience suggestions, not an exhaustive or validated list).
fn known_models(provider: &str) -> &'static [&'static str] {
    match provider.to_ascii_lowercase().as_str() {
        "ollama" => &["qwen2.5-coder:14b", "llama3.1", "mistral"],
        "openai" => &["gpt-4o", "gpt-4o-mini", "o3-mini"],
        "anthropic" => &["claude-sonnet-4", "claude-opus-4", "claude-3-5-haiku"],
        "gemini" => &["gemini-2.0-flash", "gemini-1.5-pro"],
        "openai_compatible" => &[],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_none_lists_available() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let action = state.apply(SlashCommand::Provider(None), &[]);
        if let SessionAction::Message(msg) = action {
            assert!(msg.contains("available:"));
            assert!(msg.contains("anthropic"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }

    #[test]
    fn test_model_none_lists_known_models_for_ollama() {
        let mut state = SessionState::new("test", None, "qwen2.5-coder:14b");
        state.provider = "ollama".into();
        let action = state.apply(SlashCommand::Model(None), &[]);
        if let SessionAction::Message(msg) = action {
            assert!(msg.contains("qwen2.5-coder:14b"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }

    #[test]
    fn test_provider_some_sets_provider_without_available() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let action = state.apply(SlashCommand::Provider(Some("openai".into())), &[]);
        assert_eq!(state.provider, "openai");
        if let SessionAction::Message(msg) = action {
            assert_eq!(msg, "Provider: openai");
            assert!(!msg.contains("available:"));
        } else {
            panic!("Expected SessionAction::Message");
        }
    }

    /// `/history` and `/sessions` are one command under two names: both map to
    /// `SessionAction::History` (saved sessions on disk). This is intentional
    /// aliasing, not a bug — see `slash::tests::history_help_names_the_alias`
    /// for the matching requirement that the help makes the aliasing explicit.
    #[test]
    fn history_and_sessions_map_to_the_same_action() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let history = state.apply(SlashCommand::History, &[]);
        let sessions = state.apply(SlashCommand::Sessions, &[]);
        assert_eq!(history, SessionAction::History);
        assert_eq!(sessions, SessionAction::History);
    }

    /// `/usage` returns a `SessionAction::Message` carrying the session usage
    /// breakdown. The message is handled by the existing `Message` arm in both
    /// the TUI dispatch and the headless `emit_action`, so no dispatch-layer
    /// changes were needed.
    #[test]
    fn usage_returns_breakdown_message() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        state
            .usage
            .record(&saya_agent::TokenUsage::new(100, 50).with_cached_input(Some(80)));
        let action = state.apply(SlashCommand::Usage, &[]);
        let SessionAction::Message(msg) = action else {
            panic!("expected SessionAction::Message, got {action:?}");
        };
        assert!(
            msg.contains("Input tokens: 100"),
            "breakdown must show input total, got:\n{msg}"
        );
        assert!(
            msg.contains("Output tokens: 50"),
            "breakdown must show output total, got:\n{msg}"
        );
        assert!(
            msg.contains("Cache hit rate: 80%"),
            "breakdown must show computed hit rate, got:\n{msg}"
        );
        assert!(
            msg.contains("Σcached / Σinput"),
            "breakdown must state the formula, got:\n{msg}"
        );
    }

    /// `/usage` on an empty session (no turns with usage) reports nothing
    /// rather than a row of zeros.
    #[test]
    fn usage_on_empty_session_reports_nothing() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        let action = state.apply(SlashCommand::Usage, &[]);
        let SessionAction::Message(msg) = action else {
            panic!("expected SessionAction::Message, got {action:?}");
        };
        assert!(
            msg.contains("No token usage reported yet"),
            "empty session must say no usage was reported, got:\n{msg}"
        );
    }

    /// `/clear` resets the usage accumulator along with the conversation, so a
    /// fresh context does not inherit the prior turns' token accounting.
    #[test]
    fn clear_resets_usage_accumulator() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        state.usage.record(&saya_agent::TokenUsage::new(100, 50));
        assert_eq!(state.usage.answering.turns, 1);
        state.apply(SlashCommand::Clear, &[]);
        assert_eq!(state.usage.answering.turns, 0);
        assert_eq!(state.usage.answering.input_tokens, 0);
    }

    /// `/mode` mirrors `/approvals`: bare reports without changing, a value
    /// switches, and the answer always names the current posture. Switching
    /// back to build restores the Build surface the status line reads. Both
    /// answers state the bypass composition verbatim — under `bypass`, Plan
    /// still denies writes.
    #[test]
    fn mode_reports_switches_and_restores() {
        use crate::slash::PLAN_BYPASS_SENTENCE;
        use saya_agent::AgentMode;
        let mut state = SessionState::new("test", None, "gpt-4o");
        // Bare reports the default without changing it.
        let SessionAction::Message(report) = state.apply(SlashCommand::Mode(None), &[]) else {
            panic!("expected SessionAction::Message");
        };
        assert_eq!(
            report,
            format!(
                "Mode: build — writes allowed, asks per approval policy. {PLAN_BYPASS_SENTENCE}"
            )
        );
        assert_eq!(state.agent_mode, "build");
        // Switching to plan answers plan and sticks.
        let SessionAction::Message(switched) =
            state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[])
        else {
            panic!("expected SessionAction::Message");
        };
        assert_eq!(
            switched,
            format!(
                "Mode: plan — read-only; write-shaped tools are hidden and refuse. {PLAN_BYPASS_SENTENCE}"
            )
        );
        assert_eq!(state.agent_mode, "plan");
        // Bare now reports plan.
        let SessionAction::Message(again) = state.apply(SlashCommand::Mode(None), &[]) else {
            panic!("expected SessionAction::Message");
        };
        assert_eq!(
            again,
            format!(
                "Mode: plan — read-only; write-shaped tools are hidden and refuse. {PLAN_BYPASS_SENTENCE}"
            )
        );
        // Switching back restores the Build surface.
        state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[]);
        assert_eq!(state.agent_mode_parsed(), AgentMode::Build);
        let SessionAction::Message(back) = state.apply(SlashCommand::Mode(None), &[]) else {
            panic!("expected SessionAction::Message");
        };
        assert_eq!(
            back,
            format!(
                "Mode: build — writes allowed, asks per approval policy. {PLAN_BYPASS_SENTENCE}"
            )
        );
    }

    /// Both `/mode` answers assert the bypass composition sentence verbatim —
    /// the shared constant, not a copy that can drift.
    #[test]
    fn mode_answers_state_the_bypass_composition_verbatim() {
        use crate::slash::PLAN_BYPASS_SENTENCE;
        use saya_agent::AgentMode;
        let mut state = SessionState::new("test", None, "gpt-4o");
        let SessionAction::Message(build) =
            state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[])
        else {
            panic!("expected SessionAction::Message");
        };
        let SessionAction::Message(plan) =
            state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[])
        else {
            panic!("expected SessionAction::Message");
        };
        assert!(
            build.contains(PLAN_BYPASS_SENTENCE),
            "build answer must carry the bypass sentence verbatim: {build}"
        );
        assert!(
            plan.contains(PLAN_BYPASS_SENTENCE),
            "plan answer must carry the bypass sentence verbatim: {plan}"
        );
    }

    /// `/run …` from a Plan session refuses — a run writes through its
    /// own declared capabilities rather than the session's mode, so a Plan
    /// session that spawned one would step around the posture it just set.
    /// The refusal names the mode and the remedy (`/mode build`), never
    /// silently narrowing the run, and never silently allowing it. Cancelling
    /// a run and listing runs are read-shaped and stay available.
    #[test]
    fn run_from_a_plan_session_refuses_with_mode_and_remedy() {
        use saya_agent::AgentMode;
        let mut state = SessionState::new("test", None, "gpt-4o");
        state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[]);
        let action = state.apply(
            SlashCommand::Run("survey the data --allow workspace-write".into()),
            &[],
        );
        let SessionAction::Error(message) = action else {
            panic!("a Plan session must refuse /run, got {action:?}");
        };
        assert!(
            message.contains("Plan mode"),
            "the refusal names the mode, got:\n{message}"
        );
        assert!(
            message.contains("/mode build"),
            "the refusal names the remedy, got:\n{message}"
        );
        // `/runs` and `/run cancel` are read-shaped: still available.
        let action = state.apply(SlashCommand::Runs(None), &[]);
        assert_eq!(action, SessionAction::Runs(None));
        let action = state.apply(SlashCommand::RunCancel("r-1".into()), &[]);
        assert_eq!(action, SessionAction::RunCancel("r-1".into()));
    }

    /// `/mode build` then `/run …` works exactly as today: the guard reads
    /// only Plan, so a Build session's `/run` family is byte-identical to
    /// `release/0.4.1` on every path.
    #[test]
    fn run_after_mode_build_matches_today() {
        use saya_agent::AgentMode;
        let mut state = SessionState::new("test", None, "gpt-4o");
        state.apply(SlashCommand::Mode(Some(AgentMode::Plan)), &[]);
        state.apply(SlashCommand::Mode(Some(AgentMode::Build)), &[]);
        let action = state.apply(SlashCommand::Run("survey the data".into()), &[]);
        assert_eq!(action, SessionAction::Run("survey the data".into()));
        let action = state.apply(SlashCommand::Runs(Some("r-1".into())), &[]);
        assert_eq!(action, SessionAction::Runs(Some("r-1".into())));
        let action = state.apply(SlashCommand::RunCancel("r-1".into()), &[]);
        assert_eq!(action, SessionAction::RunCancel("r-1".into()));
    }
}
