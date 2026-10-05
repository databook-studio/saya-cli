use super::session_state::SessionState;
use crate::agent::runtime::PromptOverrides;
use crate::cli::ContractsCommand;
use crate::slash::SlashCommand;
use saya_agent::AgentOutput;
use saya_agent::ApprovalPolicy;
use saya_store::FsSessionStore;
use std::path::Path;

/// `/sessions list` — the shared saved-session listing for both surfaces.
/// `Ok` is the same text the headless `saya sessions list` prints (rows
/// newest first, pending deletions named separately, the storage root
/// last); `Err` is the failure message in the headless words.
pub(crate) fn sessions_list_text(
    store: &FsSessionStore,
    sessions_root: &Path,
) -> Result<String, String> {
    let listing = super::session_resume::block_on(crate::commands::sessions::list_sessions(store))
        .map_err(|error| format!("could not list saved sessions: {error}"))?;
    Ok(crate::commands::sessions::render_listing(
        &listing,
        crate::commands::sessions::unix_ms_now(),
        sessions_root,
    ))
}

/// `/sessions delete <id>` — the shared delete outcome for both surfaces.
/// `true` when the delete completed (the REPL turn completes, the TUI
/// pushes a System block); `false` for every refusal or failure (the turn
/// errors, the TUI pushes an Error block). The message is the headless
/// `render_delete` wording, byte for byte. `current_session_id` is the
/// calling session's own id on both surfaces, so a session can never
/// delete itself underneath its owner.
pub(crate) fn sessions_delete_text(
    store: &FsSessionStore,
    sessions_root: &Path,
    id: &str,
    current_session_id: Option<&str>,
) -> (bool, String) {
    let outcome = crate::commands::sessions::delete_saved_session(
        store,
        sessions_root,
        id,
        current_session_id,
    );
    let (code, message) = crate::commands::sessions::render_delete(&outcome, id);
    (code == 0, message)
}

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
    /// `/export` — the parsed request (mode, overwrite, path) the TUI's
    /// export dispatch acts on. Headless sessions never receive it: they
    /// refuse export as TUI-only.
    Export(crate::slash::ExportRequest),
    /// `/report` — the parsed request (path, rows, overwrite) the TUI's
    /// report dispatch acts on. Headless sessions never receive it: they
    /// refuse report as TUI-only, exactly like export.
    Report(crate::slash::ReportRequest),
    Chart(String),
    Explain(String),
    /// A contract slash command, translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces. The loops hand it to the
    /// shared `run_contracts` dispatcher — no second parsing or DTO mapping.
    Contracts(ContractsCommand),
    /// A saved-investigation slash command, translated to the same
    /// `InvestigationCommand` the headless `saya investigation` parser
    /// produces. Both loops hand it to the shared `run_investigation`
    /// operation; only the TUI adds the save-without-SQL fill from the last
    /// selectable query (the headless REPL has no last query).
    Investigation(crate::cli::InvestigationCommand),
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
    /// `/context` — the instruction-source view through the shared
    /// `session_context::context_view`. Intercepted in the loop (it needs
    /// the runtime's settings and the session's root); the arm here keeps
    /// the match exhaustive.
    Context,
    /// `/sessions list` — the shared saved-session listing. Intercepted in
    /// both loops (they own the store and the sessions root); the arm here
    /// keeps the match exhaustive.
    SessionsList,
    /// `/sessions delete <id>` — the shared delete through the same
    /// operation the headless `saya sessions delete` runs, carrying the id
    /// as stated. Intercepted in both loops, which pass the session's own
    /// id as the current one; the arm here keeps the match exhaustive.
    SessionsDelete(String),
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
            SlashCommand::Tasks(subcommand) => match subcommand.as_deref() {
                // `/tasks clear` empties the stored list, so the next turn
                // injects no task block. Nothing else edits from here: the
                // model owns the content, the user owns whether it exists.
                Some("clear") => {
                    self.task_list = Default::default();
                    SessionAction::Message("Task list cleared.".into())
                }
                _ => SessionAction::Message(super::session_tasks_view::render_tasks_view(
                    &self.task_list,
                )),
            },
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
    use crate::interactive::session_resume::block_on;
    use saya_store::{RedactedMessage, RedactedSession, SessionStore};

    fn scratch(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "saya-session-commands-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

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

    /// `/export` carries the parsed request through unchanged — the TUI's
    /// export dispatch owns the behaviour; the adapter owns nothing but the
    /// mapping.
    #[test]
    fn export_request_maps_to_the_same_action() {
        use crate::slash::{ExportMode, ExportRequest};
        let mut state = SessionState::new("test", None, "gpt-4o");
        let request = ExportRequest {
            mode: Some(ExportMode::Snapshot),
            overwrite: true,
            path: "out.csv".into(),
        };
        let action = state.apply(SlashCommand::Export(request.clone()), &[]);
        assert_eq!(action, SessionAction::Export(request));
    }

    /// `/report` carries the parsed request through unchanged — the TUI's
    /// report dispatch owns the behaviour; the adapter owns nothing but the
    /// mapping.
    #[test]
    fn report_request_maps_to_the_same_action() {
        use crate::slash::ReportRequest;
        let mut state = SessionState::new("test", None, "gpt-4o");
        let request = ReportRequest {
            path: "out.md".into(),
            rows: Some(20),
            overwrite: true,
        };
        let action = state.apply(SlashCommand::Report(request.clone()), &[]);
        assert_eq!(action, SessionAction::Report(request));
    }

    /// `/investigation` carries the translated command through unchanged —
    /// both loops hand it to the shared `run_investigation` operation; the
    /// adapter owns nothing but the mapping.
    #[test]
    fn investigation_command_maps_to_the_same_action() {
        use crate::cli::InvestigationCommand;
        let mut state = SessionState::new("test", None, "gpt-4o");
        let command = InvestigationCommand::List {
            limit: None,
            offset: None,
        };
        let action = state.apply(SlashCommand::Investigation(command.clone()), &[]);
        assert_eq!(action, SessionAction::Investigation(command));
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

    /// The `/sessions` subforms map to their own actions: `list` and
    /// `delete <id>` carry the id through unchanged — the loops own the
    /// behaviour through the shared operations; the adapter owns nothing
    /// but the mapping. Bare `/sessions` stays `History`.
    #[test]
    fn sessions_subforms_map_to_their_actions() {
        let mut state = SessionState::new("test", None, "gpt-4o");
        assert_eq!(
            state.apply(SlashCommand::Sessions, &[]),
            SessionAction::History,
            "bare /sessions keeps today's meaning"
        );
        assert_eq!(
            state.apply(SlashCommand::SessionsList, &[]),
            SessionAction::SessionsList
        );
        assert_eq!(
            state.apply(SlashCommand::SessionsDelete("abc".into()), &[]),
            SessionAction::SessionsDelete("abc".into())
        );
    }

    /// `/sessions list` through the shared builder: the same listing the
    /// headless `saya sessions list` renders — rows, and the storage root
    /// as the last line; an empty store says "No saved sessions.".
    #[test]
    fn sessions_list_text_renders_the_shared_listing() {
        let root = scratch("list");
        let store = FsSessionStore::new(&root);
        block_on(store.save(RedactedSession {
            id: "other".into(),
            messages: vec![RedactedMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            ..Default::default()
        }))
        .unwrap();
        let text = sessions_list_text(&store, &root).expect("the listing renders");
        assert!(
            text.lines().any(|line| line.starts_with("other\t")),
            "the saved session is a row: {text}"
        );
        assert!(
            text.ends_with(&format!("Storage root: {}", root.display())),
            "the root is the last line: {text}"
        );
        let empty = scratch("list-empty");
        let empty_store = FsSessionStore::new(&empty);
        let text = sessions_list_text(&empty_store, &empty).expect("the empty listing renders");
        assert!(
            text.starts_with("No saved sessions."),
            "an empty store says so: {text}"
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&empty);
    }

    /// `/sessions delete` through the shared builder: a real delete says
    /// the retained-data sentence and removes the record; the current
    /// session is refused with the D09a words and nothing is touched; an
    /// unknown id says "no saved session".
    #[test]
    fn sessions_delete_text_reports_success_refusals_and_unknown_ids() {
        let root = scratch("delete");
        std::fs::write(root.join("a.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("a")).unwrap();
        let store = FsSessionStore::new(&root);
        let (ok, text) = sessions_delete_text(&store, &root, "a", Some("a"));
        assert!(!ok, "the current session is refused: {ok}");
        assert!(
            text.contains("live session") && text.contains("was not deleted"),
            "the refusal is the current-session one: {text}"
        );
        assert!(root.join("a.json").exists(), "the record survived");
        let (ok, text) = sessions_delete_text(&store, &root, "a", None);
        assert!(ok, "a foreign delete completes: {text}");
        assert!(
            text.contains("Deleted saved session a") && text.contains("were not deleted"),
            "the success carries the retained-data sentence: {text}"
        );
        assert!(!root.join("a.json").exists(), "the record is gone");
        let (ok, text) = sessions_delete_text(&store, &root, "nosuch-id", None);
        assert!(!ok, "an unknown id is refused: {text}");
        assert!(
            text.contains("no saved session nosuch-id"),
            "the unknown-id refusal names the id: {text}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
