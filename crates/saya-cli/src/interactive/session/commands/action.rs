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
    /// `/rotate` — continue the session in a small successor. Intercepted
    /// in both loops (they own the store and the runtime, and the shared
    /// `session_rotate_run` runner needs both); the arm here keeps the
    /// match exhaustive.
    Rotate,
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
    /// TUI) hand them to the shared `permissions::grants::allow`, which parses
    /// them on the session surface and seeds the session's one grant store;
    /// the loops own the policy, so the seeding lives there.
    Allow(Vec<String>),
    /// `/grants` — the session grant store listed verbatim by the shared
    /// `permissions::grants::listing`.
    Grants,
    /// `/context` — the instruction-source view through the shared
    /// `context_view::context_view`. Intercepted in the loop (it needs
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
    /// `/preferences …` — the shared project-preference operations the
    /// headless `saya preferences` runs, scoped to the session's bound
    /// worktree root. Intercepted in both loops (they own the state store
    /// and the session's root); the arm here keeps the match exhaustive.
    /// Never prompts and never a model-callable tool.
    Preferences(crate::cli::PreferencesCommand),
    /// `/recovery …` — the parsed recovery command, carried verbatim. The
    /// loops intercept it (they hold the session's recovery handle — its
    /// own flag, store and id) and hand it to the shared `run_recovery`
    /// operation; the arm here keeps the match exhaustive. Never prompts
    /// and never a model-callable tool.
    Recovery(crate::commands::recovery::RecoveryCommand),
    /// `/setup` — hand the terminal to the guided setup. The full-screen TUI
    /// runs it through the terminal-handoff boundary (`Dispatch::RunSetup`);
    /// headless sessions intercept it in the loop and refuse with the
    /// `saya setup` pointer. Never a capability: the wizard writes user
    /// configuration files only, through the existing consent flow.
    Setup,
    /// `/reload` — re-read the configuration with the launch options and
    /// apply what is safe between turns. Intercepted in both loops (they
    /// own the runtime handle and the session state, and a failed reload
    /// must leave both untouched); the arm here keeps the match exhaustive.
    /// Never prompts and never a model-callable tool.
    Reload,
    Exit,
}
