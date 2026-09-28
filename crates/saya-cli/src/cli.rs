use clap::{Args, Parser, Subcommand, ValueEnum};

const AFTER_HELP: &str = "Examples:\n  \
saya                                   start the interactive REPL\n  \
saya ask \"count orders per region\"     one-shot question\n  \
echo \"SELECT 1\" | saya query           piped SQL works too\n  \
saya query --sql \"SELECT 1\"            bounded read-only SQL\n  \
saya config doctor                     diagnose setup problems\n  \
saya completions --shell zsh > completion.zsh\n\nExit codes: 0 ok · 2 usage · 3 connection/config · 4 safety/query · 5 agent · 6 paused · 130 cancelled";

#[derive(Debug, Clone, Parser)]
#[command(
    name = "saya",
    version,
    about = "Database-aware AI for the terminal",
    after_help = AFTER_HELP
)]
pub struct Cli {
    #[command(flatten)]
    pub options: GlobalOptions,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Global options shared by every subcommand and the bare REPL.
#[derive(Debug, Clone, Args, Default)]
pub struct GlobalOptions {
    /// Continue the most recent session.
    ///
    /// Not a global flag: its only consumer is the interactive session this
    /// flag sits beside, and a run is not a session — `saya run --continue`
    /// is a usage error, never a silently ignored intent. The pre-subcommand
    /// spelling (`saya --continue run`) is refused by the dispatch guard.
    #[arg(long = "continue")]
    pub continue_session: bool,
    /// Resume a saved session by id (see `saya config doctor` / /sessions).
    #[arg(long, global = true)]
    pub resume: Option<String>,
    /// Bind the session's workspace root explicitly to this directory
    /// (canonicalised and pinned into the session). Without it the root is
    /// the git worktree top above the launch directory, and outside any
    /// worktree nothing binds: the write-shaped tools stay hidden and the
    /// workspace reads refuse.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<std::path::PathBuf>,
    /// Seed session grants at launch, stated in the `--allow` grammar.
    /// Session-only: a subcommand is not a session — `saya ask` and `saya run`
    /// refuse it rather than silently ignoring a stated intent.
    #[arg(long, value_name = "SCOPES", value_delimiter = ',')]
    pub allow: Vec<String>,
    /// Deny programs for the session, by bare name (repeatable): a
    /// user-stated refusal evaluated before every grant, every approval
    /// prompt, and bypass, at every session door that execs a program by
    /// name — `run_command`, `run_program`, the interpreter door.
    /// Refusal-only: it composes nothing. Session-only: `saya ask` and
    /// `saya run` refuse it rather than silently ignoring a stated intent.
    #[arg(long = "deny", value_name = "PROGRAM")]
    pub deny: Vec<String>,
    /// Read one turn from this file instead of stdin, run exactly that turn,
    /// then exit. The bytes reach the turn unaltered — blank lines, trailing
    /// whitespace, code fences — where piped stdin reads line by line and
    /// folds layout. Session-surface only: `saya ask` and `saya run` refuse
    /// it rather than silently ignoring a stated intent.
    #[arg(long = "turn-file", value_name = "PATH")]
    pub turn_file: Option<std::path::PathBuf>,
    /// Connection profile to use (overrides `default_profile` in config).
    #[arg(long, global = true)]
    pub profile: Option<String>,
    /// Override the configured AI model for this invocation.
    #[arg(long, global = true)]
    pub model: Option<String>,
    /// Override the configured provider (ollama|openai|openai_compatible|anthropic|gemini).
    #[arg(long, global = true)]
    pub provider: Option<String>,
    /// Override the configured row cap for query results.
    #[arg(long, value_name = "N", global = true)]
    pub max_rows: Option<usize>,
    /// Answer with the best of N independent attempts (default 1). Each attempt
    /// is a full agent run, so N attempts cost roughly N times as much.
    #[arg(long, value_name = "N", global = true)]
    pub candidates: Option<usize>,
    /// Additional profiles to query alongside the active one.
    #[arg(long = "include-profile", global = true)]
    pub include_profiles: Vec<String>,
    /// When tool calls need approval: ask | read-only | never | bypass.
    /// Read-only auto-approves read-shaped tools only; tools with external
    /// side effects are denied. `bypass` runs every call without asking;
    /// every structural guard still applies.
    #[arg(long, value_name = "MODE", global = true)]
    pub approval_mode: Option<String>,
    /// Output format for subcommands: text | json | ndjson.
    #[arg(long, value_enum, default_value_t = FormatArg::Text, global = true)]
    pub format: FormatArg,
    /// Run without any terminal interaction (CI-safe; approvals deny).
    #[arg(long, global = true)]
    pub non_interactive: bool,
    /// Explicit config.toml path (overrides user/project discovery).
    #[arg(long, global = true)]
    pub config: Option<std::path::PathBuf>,
    /// Explicit connections.toml path.
    #[arg(long, global = true)]
    pub connections: Option<std::path::PathBuf>,
    /// Explicit env file with SAYA_* overrides (never implicit .env).
    #[arg(long, global = true)]
    pub env_file: Option<std::path::PathBuf>,
    /// Enable cloud data sharing for this invocation (sharing:on), overriding any
    /// config layer that disabled it.
    #[arg(long, global = true)]
    pub allow_data_sharing: bool,
    /// Force-disable cloud data sharing for this invocation, overriding any
    /// config layer that enabled it.
    #[arg(long = "no-data-sharing", global = true)]
    pub no_data_sharing: bool,
    /// Accept security-critical settings (`ai.base_url`, `ai.api_key`,
    /// `ai.allow_data_sharing`, `run.read_only`) from the project layer's
    /// `.saya/config.toml`. Off by default: a cloned repository is untrusted.
    #[arg(long, global = true, env = "SAYA_TRUST_PROJECT_CONFIG")]
    pub trust_project_config: bool,
    /// Disable colored output (overrides the detected terminal capability).
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Colour palette for the TUI: `dark`, `light`, or `auto` (honour
    /// `COLORFGBG`, falling back to dark when the terminal reports nothing).
    #[arg(long, value_enum, default_value_t = ThemeArg::Auto, global = true)]
    pub theme: ThemeArg,
    /// Show the model's chain-of-thought in the transcript as it streams.
    /// Off by default: thinking is verbose and restates database contents in
    /// prose. Display only — reasoning is never persisted to a session file.
    #[arg(long, global = true)]
    pub show_thinking: bool,
    /// Print extraction-trace diagnostics: why a learned fact was or was not
    /// recorded after a turn (the "memory didn't record" gate).
    #[arg(long, short, global = true)]
    pub verbose: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    #[default]
    Text,
    Json,
    Ndjson,
}

/// CLI mirror of the `[ui] theme` config setting, parsed by clap from
/// `--theme <dark|light|auto>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum ThemeArg {
    #[default]
    Auto,
    Dark,
    Light,
}

impl ThemeArg {
    pub fn to_choice(self) -> saya_config::ThemeChoice {
        match self {
            Self::Auto => saya_config::ThemeChoice::Auto,
            Self::Dark => saya_config::ThemeChoice::Dark,
            Self::Light => saya_config::ThemeChoice::Light,
        }
    }
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Build (or reuse) the deterministic demo database — synthetic customers
    /// and orders designed to exercise tricky SQL — and open it read-only in
    /// an interactive session. Without a terminal (or with `--non-interactive`)
    /// it prints the database and connections paths, the launch command, and
    /// example SQL instead of launching.
    Demo {
        /// Rebuild the demo database even when a current fixture already
        /// exists on disk.
        #[arg(long)]
        reset: bool,
    },
    /// Manage saya's configuration: write starter templates, diagnose setup,
    /// or print the effective configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Inspect configured database connections: list profiles, test one, or
    /// read a profile's schema.
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// Ask one natural-language question about the database; the agent writes
    /// and runs the bounded read-only SQL itself. For raw SQL, use `saya query`.
    Ask {
        /// The question to ask. Omit it to read the question from `--file`, or
        /// from stdin when input is piped in.
        prompt: Option<String>,
        /// Read the question from this file instead of the positional
        /// `[PROMPT]`.
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    /// Run bounded read-only SQL directly against the database, bypassing the
    /// agent. For a natural-language question, use `saya ask`.
    Query {
        /// The SQL to run. Omit it to read the statement from `--file`, or from
        /// stdin when input is piped in.
        #[arg(long)]
        sql: Option<String>,
        /// Read the SQL from this file instead of `--sql`.
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    /// Inspect and manage the knowledge saya recalls: list, show, queue,
    /// remember, forget, or decide a claim.
    Contracts {
        #[command(subcommand)]
        command: ContractsCommand,
    },
    /// Manage saved investigations: save a bounded read-only SQL query as a
    /// portable JSON document (no credentials, rows, or machine state), list
    /// or show the saved definitions, replay one with `run`, export or import
    /// one as a portable JSON file, or delete one. Saving validates the SQL
    /// with the same read-only gate execution uses but never runs it, and no
    /// connection is made. Review the saved SQL before sharing: literals are
    /// stored verbatim.
    Investigation {
        #[command(subcommand)]
        command: InvestigationCommand,
    },
    /// Run a long-running, resumable job against the configured database:
    /// `saya run "<goal>"` plans and executes it step by step, pausing (never
    /// silently stopping) when a budget trips; `saya run resume|cancel|list|
    /// show|log` manage runs. Scopes must be declared up front with `--allow`:
    /// a headless run pre-authorizes them, an interactive terminal approves
    /// the bound plan, its scopes, and its budgets once.
    Run {
        /// The run's goal.
        prompt: Option<String>,
        /// Approved capability scopes, comma-separated. `none` states a
        /// deliberately read-only run — the empty scope set — and must stand
        /// alone. The wired scopes bind: `workspace-write` (writes outside
        /// reads), `scratch` (a scratch database), `fetch:<scheme>+<host>`
        /// (network fetches to that scheme and bare host), and
        /// `runner:<program>` (a program the runner executes directly).
        /// `interpreter:<program>` grants a shell or interpreter — a program
        /// that can spawn arbitrary children, so the runner will not choose
        /// one on its own; naming it here is the only way a run may use one.
        /// `sql:<connection>` seeds the run's decider with the connection's
        /// per-call grant: under `--approval-mode ask`, the read-shaped SQL
        /// tools' calls that name that connection run without asking, and a
        /// resume re-derives the grant from the run's journal.
        /// `command:<program>` is refused with a usage error: a run is
        /// unattended and this scope names unconfined host execution.
        /// `endpoint:<role>=<endpoint>` is refused with a usage error:
        /// per-step endpoint roles are not bound. A run states its scopes
        /// up front or does not start —
        /// nothing runs unapproved.
        #[arg(long, value_name = "SCOPES", value_delimiter = ',')]
        allow: Vec<String>,
        /// Budget overrides as KEY=VALUE: `wall-clock=<seconds>`,
        /// `turns=<n>`, `tool-calls=<n>`, or `tokens.orchestrator=<n>` —
        /// the only token ceiling a run accepts, since every episode calls
        /// the orchestrator endpoint. Unset keys fall back to `[jobs]`; a
        /// zero ceiling is refused as a typo, not clamped.
        #[arg(long, value_name = "KEY=VALUE")]
        budget: Vec<String>,
        #[command(subcommand)]
        command: Option<RunCommand>,
    },
    /// Generate shell completion scripts for `saya`.
    Completions {
        /// Shell to generate completions for.
        #[arg(long, value_enum)]
        shell: clap_complete::Shell,
    },
    /// Set up saya interactively: choose an AI provider and a database
    /// profile, review the exact file changes, probe the database (and, only
    /// with your consent, the provider), then write. Needs a terminal — for
    /// scripts use `saya config init` or `saya demo`.
    Setup,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Write starter config.toml and connections.toml templates to your user
    /// config directory (the trusted layer), so a following command does not
    /// warn. Pass `--project` to write this project's untrusted `.saya/` pair
    /// instead — for team-shared, non-secret settings checked into a repo.
    Init {
        /// Write to this project's `.saya/` instead of your user config
        /// directory. The project layer is untrusted, so a command run
        /// afterward warns until you pass `--trust-project-config`.
        #[arg(long)]
        project: bool,
    },
    /// Diagnose configuration: secrets resolve? what provider endpoint is
    /// configured? Exits non-zero (3) when the setup cannot run a query, so a
    /// script can tell.
    Doctor,
    /// Print the effective (redacted) configuration as JSON.
    Show,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConnectionCommand {
    /// List configured connection profiles.
    List,
    /// Connect to a profile and report success/latency.
    Test {
        /// Name of the connection profile to test.
        #[arg(value_name = "PROFILE")]
        profile_name: String,
    },
    /// Print a profile's cached schema, or re-discover it live with `--refresh`.
    Schema {
        /// Name of the connection profile whose schema to read.
        #[arg(value_name = "PROFILE")]
        profile_name: String,
        /// Re-discover the schema live and refresh the cache, instead of reading
        /// the cached copy.
        #[arg(long)]
        refresh: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ContractsCommand {
    /// List every recalled contract (claim) for a profile.
    List {
        /// Profile whose claims to list; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show one object's contract — every claim saya recalls for that table.
    Show {
        /// The object to show, as `catalog.schema.table` (or `schema.table`).
        table: String,
        /// Profile whose claim on this object to show; defaults to the active
        /// profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List candidate claims awaiting your review (the worklist, not the
    /// archive).
    Queue {
        /// Profile whose candidates to queue; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Maximum candidates to list. Clamped to 200; a queue is a worklist,
        /// not an archive.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Store a confirmed claim about a table, so recall surfaces it on later
    /// turns. For directive kinds, `--reason` records why it holds.
    Remember {
        /// The object the claim is about, as `catalog.schema.table`.
        table: String,
        /// Kind of claim to store.
        #[arg(long, value_enum)]
        kind: ClaimKindArg,
        /// The claim's value (a description, an alias, a column name, a role…).
        #[arg(long)]
        value: String,
        /// The column a column-scoped claim is about (for `column-description`
        /// and `column-role` kinds).
        #[arg(long)]
        column: Option<String>,
        /// Why the directive claim holds — a sentence the model reads alongside
        /// the value so a claim that contradicts a plausible schema reading
        /// (use `return_date`, not `rental_date`) loses less often. Forwarded to
        /// the directive kinds only (grain, time-column, column-role); ignored
        /// for description/alias. Optional: a claim with no reason is the default.
        #[arg(long)]
        reason: Option<String>,
        /// Profile to store the claim against; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Confirm or reject a claim, naming it by the short `ki-xxxx` prefix that
    /// `contracts list` prints. A prefix matching no claim, or more than one,
    /// is refused and changes nothing.
    //
    // Implementation note, deliberately not a doc comment: clap prints doc
    // comments verbatim in `--help`, so anything here is user-facing. The
    // prefix resolves against the resolved profile's claims and the decision
    // then reaches the existing confirm/reject/use_candidate_once operations —
    // this is not a second implementation of them.
    Decide {
        /// A leading prefix of a stored claim id. Unambiguous-or-refused: zero
        /// matches or more than one is a typed error that changes nothing.
        prefix: String,
        /// The decision to record: `confirm`, `reject`, or `use-once` (admit
        /// a candidate for one recall without promoting it).
        #[arg(long, value_enum)]
        decision: ReviewDecisionArg,
        /// Profile whose claim to decide; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Confirm every candidate claim in the review queue — the same bounded
    /// set `contracts queue` shows. Every candidate still goes through the
    /// per-item validation `contracts decide --confirm` applies, so a batch is
    /// expected to be a mixture: an item dismissed since it was queued, or one
    /// whose object the cached schema can no longer vet, is refused. Every
    /// approved item and every refusal (with its reason) is reported by claim
    /// id; approved items are never rolled back. Exits 0 when anything was
    /// approved (or nothing was waiting) and 2 when every item was refused.
    /// Without `--yes` the queue is printed and nothing is approved.
    //
    // Implementation note, deliberately not a doc comment: clap prints doc
    // comments verbatim in `--help`, so anything here is user-facing. This
    // variant routes through the same `confirm()` the single-item decide path
    // uses — the batch adds no validation and removes none.
    ApproveAll {
        /// Profile whose queue to approve; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Maximum candidates to approve. Clamped to 200, exactly like
        /// `contracts queue` — this approves the queue you were shown, not the
        /// whole archive.
        #[arg(long)]
        limit: Option<usize>,
        /// Approve. Without it the command prints the queue and approves
        /// nothing (the same deny-by-default the `--non-interactive` approval
        /// policy applies); with it the per-item sweep runs and reports.
        #[arg(long)]
        yes: bool,
    },
    /// Tombstone a claim so recall stops surfacing it.
    Forget {
        /// The stored claim id to forget (the full id, or a `ki-xxxx` prefix
        /// that names one claim).
        claim_id: String,
        /// Why the claim is being forgotten.
        #[arg(long, value_enum, default_value_t = ForgetReasonArg::UserRequest)]
        reason: ForgetReasonArg,
    },
}

/// Saved-investigation subcommands. The same enum the slash adapter (S9)
/// will translate into, so the clap surface and the TUI cannot drift apart.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum InvestigationCommand {
    /// Save a bounded read-only SQL query as a portable investigation
    /// document: one JSON file with the exact SQL and nothing else — no
    /// credentials, rows, results, or machine-specific identity. The SQL is
    /// validated by the same read-only gate execution uses but is never
    /// executed here, and the saving profile is recorded locally as the
    /// review binding. Review the saved SQL before sharing: literals are
    /// stored verbatim.
    Save {
        /// Name of the investigation, shown in `investigation list`.
        #[arg(long)]
        name: String,
        /// Optional description of what the investigation answers.
        #[arg(long)]
        description: Option<String>,
        /// The exact SQL to save. Omit it to read the statement from
        /// `--file`, or from stdin when input is piped in.
        #[arg(long)]
        sql: Option<String>,
        /// Read the SQL from this file instead of `--sql`.
        #[arg(long = "file", value_name = "PATH")]
        file: Option<std::path::PathBuf>,
        /// Connection profile to save against; defaults to the active profile.
        #[arg(long, value_name = "PROFILE")]
        connection: Option<String>,
    },
    /// List saved investigations, one line each, at most 50 per page.
    List {
        /// Maximum entries to list, 1-50 (default 50).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Skip this many entries before listing.
        #[arg(long, value_name = "N")]
        offset: Option<usize>,
    },
    /// Print one saved investigation's exact definition as JSON, plus the
    /// local review binding for this machine. The opaque profile identity is
    /// never printed.
    Show {
        /// The id of the investigation to show (`investigation list` prints
        /// the ids).
        id: String,
    },
    /// Delete one saved investigation and its local binding. The current
    /// revision is checked first, so a document changed underneath this
    /// command is refused rather than deleted unseen.
    Delete {
        /// The id of the investigation to delete (`investigation list`
        /// prints the ids).
        id: String,
        /// Require the document to be at this revision; omit it to delete
        /// the revision currently on disk.
        #[arg(long, value_name = "N")]
        revision: Option<u32>,
    },
    /// Write one saved investigation's portable definition to a file for
    /// sharing. Only the definition travels: the per-machine local review
    /// binding is never included. The destination must not exist unless
    /// `--overwrite` is passed, and the write is atomic — a refused or
    /// failed export leaves any existing destination unchanged. The exact
    /// SQL is stored verbatim; review it before sharing.
    Export {
        /// The id of the investigation to export (`investigation list`
        /// prints the ids).
        id: String,
        /// Destination file path. An existing file needs `--overwrite`; a
        /// directory or symlink destination is refused.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
        /// Replace an existing destination file. Without it an existing
        /// destination is refused, never clobbered.
        #[arg(long)]
        overwrite: bool,
    },
    /// Read a portable investigation definition file written by `investigation
    /// export`, validate the whole document, print a preview, and store it
    /// with no local connection binding. Nothing is executed and no
    /// connection is made: running it requires an explicit
    /// `investigation run --connection <profile>` later.
    Import {
        /// Path to the definition file to import.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
    },
    /// Replay a saved investigation against an explicit local connection:
    /// the same bounded, read-only query path as `saya query`, with no AI
    /// provider involved. The target comes only from `--connection` or the
    /// stored local review binding — never the active or default profile —
    /// and the review must still match the definition, the target, and the
    /// referenced objects' schema, or the run is refused.
    Run {
        /// The id of the investigation to run (`investigation list` prints
        /// the ids).
        id: String,
        /// Connection profile to replay against. Required on the first run
        /// of an investigation that has no local review binding on this
        /// machine (e.g. one that was imported); the mapping it creates is
        /// remembered so later runs need no flag.
        #[arg(long, value_name = "PROFILE")]
        connection: Option<String>,
        /// Accept the current state and rewrite the local review binding:
        /// use this when a revision, target, or schema change is expected
        /// and reviewed.
        #[arg(long)]
        revalidate: bool,
        /// Write the Markdown report for this replay's result to a file:
        /// the same report the TUI's `/report` writes — SQL and provenance
        /// by default, no rows. Written only after a successful replay; a
        /// refused or failed write reports on stderr and exits 2, never
        /// touching an existing destination without `--overwrite`.
        #[arg(long, value_name = "PATH")]
        report: Option<std::path::PathBuf>,
        /// Include up to this many result rows in the report's table (at
        /// most 100); omit the flag to write SQL and provenance only.
        #[arg(long, value_name = "N")]
        rows: Option<usize>,
        /// Replace an existing report destination. Without it an existing
        /// destination is refused, never clobbered.
        #[arg(long)]
        overwrite: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum RunCommand {
    /// Continue a paused (or crashed) run at its first incomplete step. The
    /// run's journal is the authority; a run with a live holder refuses.
    Resume {
        /// The id of the run to resume (`saya run list` prints them).
        run_id: String,
    },
    /// Record a run cancelled. A run with a live holder refuses — cancel the
    /// process that owns it (Ctrl-C) instead; a finished run changes nothing.
    Cancel {
        /// The id of the run to cancel.
        run_id: String,
    },
    /// List every run, most recent first.
    List,
    /// Show one run's status, goal, scopes, pause reason, and the
    /// deliverables its steps recorded.
    Show {
        /// The id of the run to show.
        run_id: String,
    },
    /// Print one run's journal — every lifecycle and step event, in order.
    Log {
        /// The id of the run whose journal to print.
        run_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ClaimKindArg {
    Description,
    Alias,
    Grain,
    ColumnDescription,
    ColumnRole,
    TimeColumn,
}

impl ClaimKindArg {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Description => "description",
            Self::Alias => "alias",
            Self::Grain => "grain",
            Self::ColumnDescription => "column-description",
            Self::ColumnRole => "column-role",
            Self::TimeColumn => "time-column",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ForgetReasonArg {
    UserRequest,
    Incorrect,
    Obsolete,
    Privacy,
}

/// The decision a `/confirm`, `/reject`, or `/use` short-reference command
/// carries, resolved by `run_contracts` against the stored claim the prefix
/// names. Spec D. `Confirm` and `Reject` reach the existing mutating ops; `UseOnce`
/// reaches `use_candidate_once`, which validates and admits for one recall
/// without promoting — a candidate stays a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReviewDecisionArg {
    Confirm,
    Reject,
    UseOnce,
}
