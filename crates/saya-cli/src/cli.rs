use clap::{Args, Parser, Subcommand, ValueEnum};

const AFTER_HELP: &str = "Examples:\n  \
saya                                   start the interactive REPL\n  \
saya ask \"count orders per region\"     one-shot question\n  \
echo \"SELECT 1\" | saya query           piped SQL works too\n  \
saya query --sql \"SELECT 1\"            bounded read-only SQL\n  \
saya config doctor                     diagnose setup problems\n  \
saya completions --shell zsh > completion.zsh\n\nExit codes: 0 ok · 2 usage · 3 connection/config · 4 safety/query · 5 agent · 130 cancelled";

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
    #[arg(long = "continue", global = true)]
    pub continue_session: bool,
    /// Resume a saved session by id (see `saya config doctor` / /sessions).
    #[arg(long, global = true)]
    pub resume: Option<String>,
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
    /// Additional profiles to query alongside the active one.
    #[arg(long = "include-profile", global = true)]
    pub include_profiles: Vec<String>,
    /// When tool calls need approval: ask | read-only | never.
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
    /// Enable cloud data sharing for this invocation (privacy:on), overriding any
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
    /// Generate shell completion scripts for `saya`.
    Completions {
        /// Shell to generate completions for.
        #[arg(long, value_enum)]
        shell: clap_complete::Shell,
    },
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
    /// Diagnose configuration: secrets resolve? provider reachable? Exits
    /// non-zero (3) when the setup cannot run a query, so a script can tell.
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
