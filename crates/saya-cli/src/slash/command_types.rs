#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Coding(crate::cli::CodingCommand),
    Connect(String),
    Connections,
    Include(String),
    Exclude(String),
    Provider(Option<String>),
    Model(Option<String>),
    Privacy(Option<bool>),
    Approvals(Option<ApprovalPolicy>),
    Mode(Option<AgentMode>),
    Schema(bool),
    Sql(String),
    /// `/export [--snapshot|--refresh] [--overwrite] <path>` — what to
    /// write and where, parsed once here so every adapter sees the same
    /// request. No mode flag is the legacy refresh.
    Export(ExportRequest),
    /// `/report [--rows N] [--overwrite] <path>` — write a shareable
    /// Markdown report of the captured `/sql` result: SQL + provenance by
    /// default, rows only on request. Parsed once here like `/export` so
    /// every adapter sees the same request.
    Report(ReportRequest),
    Chart(String),
    Explain(String),
    Clear,
    Compact,
    History,
    Sessions,
    /// `/sessions list` — the saved-session listing through the shared
    /// operations the headless `saya sessions list` uses: rows newest
    /// first, pending deletions named separately, the storage root last.
    SessionsList,
    /// `/sessions delete <id>` — one saved session deleted through the
    /// same shared operation the headless `saya sessions delete` runs.
    /// The loops pass the session's own id as the current one, so the
    /// session can never delete itself; the form never prompts.
    SessionsDelete(String),
    /// `/preferences …` — a project-preference command, already translated
    /// to the same `PreferencesCommand` the headless `saya preferences`
    /// parser produces. Both loops hand it to the shared `run_in_session`
    /// operation scoped to the session's bound worktree root — no second
    /// parsing, DTO, or rendering; the form never prompts and never
    /// becomes a model-callable tool.
    Preferences(PreferencesCommand),
    Resume(String),
    /// `/rotate` — continue the session in a new, smaller successor that
    /// keeps the saved summary, the notes, and the not-yet-summarised tail;
    /// grants, approvals and the recovery setting are never inherited. The
    /// loops own the behaviour (they hold the store and the runtime); the
    /// form takes no argument.
    Rotate,
    /// `/setup` — hand the terminal to the guided setup from the full-screen
    /// TUI. Parsed bare; trailing text is a usage error, never an argument.
    /// The TUI owns the handoff; headless surfaces refuse with the pointer
    /// to run `saya setup` from the shell.
    Setup,
    /// `/reload` — re-read the configuration with the launch options and
    /// apply what is safe between turns. The loops own the behaviour (they
    /// hold the runtime handle and the session state); the form takes no
    /// argument. Data sharing, host commands, and the approval mode never
    /// change for a running session: the plan classifies them as pending.
    Reload,
    /// Choose which columns wide result tables show in the TUI:
    /// `/columns name1,name2` filters; `/columns` or `/columns all` resets.
    Columns(Option<String>),
    /// A contract slash command (`/contracts`, `/contract`, `/remember`,
    /// `/forget`), already translated to the same `ContractsCommand` the
    /// headless `saya contracts` parser produces. The adapter slice (2b-4)
    /// hands it to the shared `run_contracts` dispatcher — no second parsing.
    Contracts(ContractsCommand),
    /// A saved-investigation slash command (`/investigation …`, and
    /// `/investigations` as `list`), already translated to the same
    /// `InvestigationCommand` the headless `saya investigation` parser
    /// produces. Both the headless REPL and the TUI hand it to the shared
    /// `run_investigation` operation — no second parsing, store, or
    /// rendering; only the TUI's save-without-SQL fill is adapter-specific.
    Investigation(InvestigationCommand),
    /// `/run <tail>` — start or operate a headless run from the session. The
    /// raw tail is handed to a nested `saya run` child process verbatim, whose
    /// output passes through the parent's stdout/stderr unmangled (the
    /// dual-tag hazard is documented where the child spawns,
    /// `interactive::session::run`). The child's own CLI parser stays the
    /// authority on `--allow`, `--budget`, and the `cancel`/`resume`/`show`
    /// subcommands — the adapter parses nothing twice, except its one word:
    /// a leading `--seed-grants` requests that the child's `--allow` be
    /// seeded from this session's grants (accepted tokens forwarded, refused
    /// ones named), and is stripped before the child parses.
    Run(String),
    /// `/run cancel <id>` — record a run cancelled through the same engine
    /// path `saya run cancel` uses, via the shared dispatcher.
    RunCancel(String),
    /// `/runs [id]` — list every run, or show one run when you name its id,
    /// through the same read path the headless `saya run list|show` commands
    /// use.
    Runs(Option<String>),
    /// `/allow <scopes…>` — seed pre-authorisation into the session's grant
    /// store: the same `--allow` grammar, judged on the session surface.
    /// The tokens are carried as stated; the shared `session_grants`
    /// behaviour does the parsing, the seeding, and the message (`none`
    /// keeps its grammar meaning: the empty approval, alone — it seeds
    /// nothing and is not a revoke).
    Allow(Vec<String>),
    /// `/grants` — the session's grant store listed verbatim: one token per
    /// line, sorted, under a header stating the lifetime.
    Grants,
    /// `/context` — the instruction sources as they would load for the next
    /// request: per source the origin, file name, whole-file size, bytes
    /// included, short digest, truncation and redaction flags; per skipped
    /// source the origin and reason; per omitted source the origin and the
    /// budget reason. Never the file text. Parsed bare; trailing text is
    /// ignored like `/grants`.
    Context,
    /// Run `config doctor` in-session: secrets resolve? provider endpoint?
    Doctor,
    /// Show session token usage totals and cache hit rate.
    Usage,
    /// Toggle display of the model's chain-of-thought in the transcript.
    Thinking(Option<bool>),
    /// Show the session's workspace binding: the pinned canonical root, or
    /// the no-root shape where nothing binds.
    Workspace,
    Help(Option<String>),
    /// `/tasks [clear]` — the user's view of the session task list: bare
    /// shows it through the shared renderer, `clear` empties it. The model
    /// owns the list's content; no per-task editing lives here.
    Tasks(Option<String>),
    /// `/notes [set <text>|clear]` — the user's own notes on this session's
    /// continuation record: bare or `show` displays them, `set <text>`
    /// stores them (text verbatim: outer ends trimmed, interior spacing
    /// kept), `clear` removes them. User guidance on the saved summary
    /// only — never the model's, never an authority; separate from
    /// `/preferences` (per worktree) and `/remember` (database knowledge).
    /// Translated once to the same `NotesCommand` the shared `run_notes`
    /// operation runs on both surfaces.
    Notes(NotesCommand),
    /// `/recovery [on|off|list|diff <id>|restore <id>]` — the session's
    /// opt-in file-edit recovery: bare (or `status`) shows the flag and
    /// quota use, `on`/`off` flip the session flag, `list` lists the
    /// recorded edits, and `diff <id>`/`restore <id>` preview and undo one
    /// recorded edit. Translated once to the `RecoveryCommand` the shared
    /// `run_recovery` operation runs on every surface.
    Recovery(RecoveryCommand),
    Exit,
}
