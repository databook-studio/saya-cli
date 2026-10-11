use super::{
    ConfigCommand, ConnectionCommand, ContractsCommand, InvestigationCommand, McpCommand,
    PreferencesCommand, RunCommand, SessionsCommand,
};
use clap::Subcommand;

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Native coding jobs with bounded scoped approval and isolated managed checkouts.
    Coding {
        /// Original repository to snapshot; never a child-provided path.
        #[arg(long, default_value = ".")]
        repository: std::path::PathBuf,
        /// Trusted directory containing installed programs for confined commands.
        #[arg(long, default_value = "/usr/bin")]
        program_dir: std::path::PathBuf,
        #[command(subcommand)]
        command: super::CodingCommand,
    },
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
    /// Open a local CSV or Parquet file as a read-only session: the file is
    /// staged once into a private DuckDB snapshot keyed by its content hash,
    /// a preview prints, and the TUI opens over it — the same bounded,
    /// read-only query, evidence, export, and investigation machinery as any
    /// database, with no database server and no config editing. The same
    /// content reuses its snapshot; changed content stages a new one.
    /// `--list` shows staged sources; `--cleanup <SHA-PREFIX>|all` removes
    /// them (only saya's own). Non-TTY runs print the preview and the launch
    /// command and exit.
    Open {
        /// The CSV or Parquet file to open. Omit it when using `--list` or `--cleanup`.
        file: Option<std::path::PathBuf>,
        /// Field delimiter as a single ASCII character (default: sniffed from
        /// the file's first line).
        #[arg(long, value_name = "C")]
        delimiter: Option<String>,
        /// Treat the first row as data, not a header (columns become
        /// `column_1`, `column_2`, …).
        #[arg(long)]
        no_header: bool,
        /// Restage even when a snapshot of this exact content already exists.
        #[arg(long)]
        reset: bool,
        /// Also build a typed copy of the table (`<table>_typed`) by explicit
        /// `TRY_CAST` per inferred non-text column; the text table stays.
        /// Values that fail the cast are counted and reported.
        #[arg(long)]
        typed: bool,
        /// List staged file sources instead of opening one.
        #[arg(long)]
        list: bool,
        /// Remove staged snapshots: `all`, or a SHA-256 prefix naming exactly
        /// one. Only directories holding a valid staged source are touched.
        #[arg(long, value_name = "SHA_PREFIX|all")]
        cleanup: Option<String>,
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
    /// Manage saved interactive sessions: list the stored sessions with
    /// their sizes, or delete one explicitly. Never automatic, never
    /// prompted.
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    /// Manage explicit project preferences: per-worktree guidance you
    /// confirmed yourself, stored separately from database knowledge.
    /// Preferences are guidance, never permissions.
    Preferences {
        #[command(subcommand)]
        command: PreferencesCommand,
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
    /// Serve the Model Context Protocol over stdio (ADR 0008): a client
    /// (the MCP Inspector or any MCP host) speaks newline-delimited
    /// JSON-RPC on this process's stdin/stdout — protocol frames only, logs on
    /// stderr, exit on stdin EOF. The toolset is bounded to the startup
    /// allowlist: the `--profile` values, else the configured default profile;
    /// `list_profiles` reports names and dialects only, and no client can
    /// widen the set. Row-returning tools additionally need data sharing to
    /// be allowed (`--allow-data-sharing` or config).
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}
