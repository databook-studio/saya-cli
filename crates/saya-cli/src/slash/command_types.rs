#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// The job surface: `/job <verb>`, `/start`, and the seven top-level
    /// spellings that are `/job`'s verbs for one release (`/approve`, `/deny`,
    /// `/message`, `/pause`, `/continue <id> <agent>`, `/cancel-child`,
    /// `/follow-up`). Every spelling carries the same `CodingCommand` the CLI
    /// parses, so the two surfaces cannot drift; a deprecated spelling also
    /// carries its alias, for the note it owes once per session.
    Coding(JobRequest),
    /// Bare `/continue` — continue the session's last turn when it paused. A
    /// guard stop is a pause the operator's next message continues, and this is
    /// that message: the literal `Continue.`, sent through the same path a
    /// typed line uses, once (approved decision 11). The argument spelling
    /// `/continue <id> <agent>` is [`JobAlias::Continue`], a job control.
    ContinueTurn,
    Connect(String),
    Connections,
    Include(String),
    Exclude(String),
    Provider(Option<String>),
    Model(Option<String>),
    Privacy(Option<bool>),
    /// `/mode [<value>]` — one statement of the session's consent axis, and
    /// the spelling that stated it. `/approvals` is the one-release alias, so
    /// it produces **this** variant: one operation, two spellings, one state.
    Mode(ModeRequest),
    Schema(bool),
    Sql(String),
    /// `/export [--snapshot|--refresh] [--overwrite] [--rows N]
    /// [--format csv|json|md] <path>` — what to write and where, parsed once
    /// here so every adapter sees the same request. No mode flag is the
    /// legacy refresh. The extension or `--format` selects the file: `.md`
    /// (or `--format md`) writes today's Markdown report, `.csv`/`.json` the
    /// data file. `/report <args>` is this command's one-release alias, so it
    /// produces **this** variant with [`ExportRequest::via_report_alias`]
    /// set: one operation, one writer, two spellings.
    Export(ExportRequest),
    Chart(String),
    Explain(String),
    Clear,
    /// `/compact [--new-session]` — shrink working memory through the one
    /// compaction operation (AC's chat compaction), and with `--new-session`
    /// continue in a smaller successor afterwards. `/rotate` is this command's
    /// one-release alias, so it produces **this** variant with
    /// [`CompactRequest::via_rotate_alias`] set: one operation, one path.
    Compact(CompactRequest),
    /// `/sessions` — the saved-session listing, browsing through the adapter's
    /// own store. `/history` is this command's one-release alias, so it
    /// produces **this** variant with [`SessionsRequest::via_history_alias`]
    /// set: one listing, one path.
    Sessions(SessionsRequest),
    /// `/status` — the one panel: five sections built from the facts this
    /// session already holds, and never a probe (`/doctor` is the command that
    /// checks). `/usage`, `/workspace` and `/context` are its one-release
    /// aliases, each opening the one section it names.
    Status,
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
    /// A contract slash command (`/contracts <verb>`, `/remember`, `/forget`,
    /// and the one-release spellings `/contract`, `/queue`, `/confirm`,
    /// `/reject`, `/approve-all`), already translated to the same
    /// `ContractsCommand` the headless `saya contracts` parser produces, plus
    /// the deprecation note a deprecated spelling states. The adapter hands it
    /// to the shared `run_contracts_request` dispatcher — no second parsing.
    Contracts(ContractsRequest),
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
    /// `/permissions` — one read-only view of what this session may do:
    /// the mode in force, the grant store, the deny list, the host lane, and
    /// data sharing. `/grants` is its one-release alias, so it produces
    /// **this** variant with [`PermissionsRequest::via_grants_alias`] set:
    /// one view, one rendering, two spellings.
    Permissions(PermissionsRequest),
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
    /// `/undo` — restore the files the last turn edited, from the run's own
    /// checkpoint taken before that turn. One operation with
    /// `/rewind <id>` and `saya job rewind`; the checkpoint store is the
    /// run's, never the session's per-edit store. Parsed bare: trailing text
    /// is a usage error.
    Undo,
    /// `/rewind [<id>]` — bare lists the run's checkpoints newest first; one
    /// id restores that checkpoint. The id is carried as stated and resolved
    /// against the listing, so an id the run never checkpointed is refused by
    /// name.
    Rewind(RewindRequest),
    Exit,
}

/// One statement of the session's consent axis: the value `/mode` states, or
/// the read-back when none is stated, and which spelling stated it.
///
/// `/mode` and its one-release alias `/approvals` both parse to this, so the
/// operation is stated once — the switch, the bypass gate and the frozen-job
/// refusal cannot answer two spellings differently — while the surface that
/// runs it can still say that the alias is the alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModeRequest {
    /// The value stated, in the one six-value grammar; `None` reads it back.
    pub value: Option<PermissionMode>,
    /// The deprecated `/approvals` spelling stated it (one release only).
    pub via_approvals_alias: bool,
}

impl ModeRequest {
    /// A statement through the canonical `/mode` spelling.
    pub(crate) const fn mode(value: Option<PermissionMode>) -> Self {
        Self {
            value,
            via_approvals_alias: false,
        }
    }

    /// A statement through the deprecated `/approvals` alias.
    pub(crate) const fn alias(value: Option<PermissionMode>) -> Self {
        Self {
            value,
            via_approvals_alias: true,
        }
    }
}
