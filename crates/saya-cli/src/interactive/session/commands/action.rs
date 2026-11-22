// `Agent(AgentOutput)` carries agent events backed by `serde_json::Value`, so
// this enum is `PartialEq` but not `Eq`.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionAction {
    Coding(crate::slash::JobRequest),
    Message(String),
    /// `/mode <mode>` — a switch, carried to the loops. The switch needs the
    /// session's one policy, the journal, and the launch's bypass gate, so it
    /// runs through the one shared `permissions::mode::switch` the keyboard
    /// cycle runs, never in this pure state layer. The request carries the
    /// spelling that stated it, so the `/approvals` alias's deprecation note is
    /// said by the one switch. The arm here keeps the match exhaustive; an
    /// unhandled `Mode` is a bug, not a silent no-op.
    Mode(crate::slash::ModeRequest),
    /// `/compact` — shrink working memory through a bounded summariser call.
    /// The loops intercept this (it needs the provider runtime); the arm here
    /// keeps the match exhaustive.
    Compact,
    /// `/status` — the one panel. Intercepted in both loops, which hold the
    /// runtime's settings, the instruction sources and the store's record
    /// size; the arm here keeps the match exhaustive. Nothing here probes —
    /// `/doctor` is the command that checks.
    Status,
    Agent(AgentOutput),
    Cancelled,
    NotImplemented(String),
    Error(String),
    /// `/sessions` — the saved-session listing, through the adapter's own
    /// store. The flag says the one-release `/history` alias stated it, so
    /// the one listing carries that spelling's note.
    History(bool),
    /// Run `config doctor` and surface its report as a message.
    Doctor,
    Resume(String),
    /// `/compact --new-session` (and its one-release alias `/rotate`) —
    /// continue the session in a small successor. Intercepted in both loops
    /// (they own the store and the runtime, and the shared
    /// `session_rotate_run` runner needs both); the arm here keeps the match
    /// exhaustive. The request carries the spelling, so the alias's note is
    /// said by the one rotation.
    Rotate(crate::slash::CompactRequest),
    Schema(bool),
    Sql(String),
    /// `/export` — the parsed request (mode, format, rows, overwrite, path)
    /// the TUI's export dispatch acts on: a `.csv`/`.json` destination
    /// writes the rows, a `.md` one (or `--format md`) writes today's
    /// Markdown report. `/report` is the one-release alias of that
    /// `--format md` form. Headless sessions never receive it: they refuse
    /// export as TUI-only.
    Export(crate::slash::ExportRequest),
    Chart(String),
    Explain(String),
    /// A contract slash command, translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces, plus the deprecation note a
    /// deprecated spelling states. The loops hand it to the shared
    /// `run_contracts_request` seam — no second parsing or DTO mapping.
    Contracts(crate::slash::ContractsRequest),
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
    /// `/permissions` — one read-only view: the mode, the grant store, the
    /// deny list, the host lane and data sharing, through the shared
    /// `chat::ops_permissions` renderer. `/grants` carries the same request
    /// with its alias spelling, so its note rides the one view. Intercepted
    /// in both loops (they hold the session's policy, its grant store and the
    /// runtime's deny list); the arm here keeps the match exhaustive.
    Permissions(crate::slash::PermissionsRequest),
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
