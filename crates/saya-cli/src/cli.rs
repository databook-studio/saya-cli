use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(name = "saya", version, about = "Database-aware AI for the terminal")]
pub struct Cli {
    #[command(flatten)]
    pub options: GlobalOptions,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct GlobalOptions {
    #[arg(long = "continue", global = true)]
    pub continue_session: bool,
    #[arg(long, global = true)]
    pub resume: Option<String>,
    #[arg(long, global = true)]
    pub profile: Option<String>,
    #[arg(long = "include-profile", global = true)]
    pub include_profiles: Vec<String>,
    #[arg(long, value_name = "MODE", global = true)]
    pub approval_mode: Option<String>,
    #[arg(long, value_enum, default_value_t = FormatArg::Text, global = true)]
    pub format: FormatArg,
    #[arg(long, global = true)]
    pub non_interactive: bool,
    #[arg(long, global = true)]
    pub config: Option<std::path::PathBuf>,
    #[arg(long, global = true)]
    pub connections: Option<std::path::PathBuf>,
    #[arg(long, global = true)]
    pub env_file: Option<std::path::PathBuf>,
    #[arg(long, global = true)]
    pub allow_data_sharing: bool,
    #[arg(long, global = true)]
    pub no_color: bool,
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

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Connection {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    Ask {
        prompt: Option<String>,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    Query {
        #[arg(long)]
        sql: Option<String>,
        #[arg(long)]
        file: Option<std::path::PathBuf>,
    },
    Contracts {
        #[command(subcommand)]
        command: ContractsCommand,
    },
    Preferences {
        #[command(subcommand)]
        command: PreferencesCommand,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    Init,
    Doctor,
    Show {
        #[arg(long)]
        resolved: bool,
        #[arg(long)]
        redacted: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConnectionCommand {
    List,
    Test {
        #[arg(value_name = "PROFILE")]
        profile_name: String,
    },
    Schema {
        #[arg(value_name = "PROFILE")]
        profile_name: String,
        #[arg(long)]
        refresh: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ContractsCommand {
    List {
        #[arg(long)]
        profile: Option<String>,
    },
    Show {
        table: String,
        #[arg(long)]
        profile: Option<String>,
    },
    Queue {
        #[arg(long)]
        profile: Option<String>,
        /// Maximum candidates to list. Clamped to 200; a queue is a worklist,
        /// not an archive.
        #[arg(long)]
        limit: Option<usize>,
    },
    Remember {
        table: String,
        #[arg(long, value_enum)]
        kind: ClaimKindArg,
        #[arg(long)]
        value: String,
        #[arg(long)]
        column: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    Review {
        claim_id: String,
        #[arg(long)]
        confirm: bool,
        #[arg(long)]
        reject: bool,
    },
    /// Act on a claim from the turn that just showed it, by a short stored
    /// claim-id prefix (the `c-xxxx` `contracts list` abbreviates to), not a
    /// 64-character id. Spec D. The `prefix` is resolved against the resolved
    /// profile's claims to exactly one claim, or refused; the decision then
    /// reaches the existing `confirm`/`reject`/`use_candidate_once` operations
    /// — it is not a second implementation of them.
    Decide {
        /// A leading prefix of a stored claim id. Unambiguous-or-refused: zero
        /// matches or more than one is a typed error that changes nothing.
        prefix: String,
        #[arg(long, value_enum)]
        decision: ReviewDecisionArg,
        #[arg(long)]
        profile: Option<String>,
    },
    Forget {
        claim_id: String,
        #[arg(long, value_enum, default_value_t = ForgetReasonArg::UserRequest)]
        reason: ForgetReasonArg,
    },
    /// Import team contract files from `.saya/contracts/` into the store.
    Import {
        /// Project root to discover `.saya/contracts/` under. Defaults to the
        /// current directory.
        #[arg(default_value = ".")]
        path: std::path::PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Export this profile's confirmed claims to discovered-shape files.
    Export {
        /// Destination directory. One `.toml` per object is written here.
        destination: std::path::PathBuf,
        #[arg(long)]
        profile: Option<String>,
        /// Overwrite an existing destination file. Without this flag an existing
        /// file is a typed error, not a silent overwrite.
        #[arg(long)]
        force: bool,
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

/// `saya preferences` — the four kinds a preference value can be. The value's
/// own `required_scope` decides the scope; this enum carries only the kind, so
/// `set`/`unset` share one vocabulary with the slash path and no second one
/// exists. Kebab-case to match `saya preferences set <kind>` in the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PreferenceKindArg {
    Timezone,
    DateGrain,
    OutputStyle,
    DefaultProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum PreferencesCommand {
    List {
        #[arg(long)]
        profile: Option<String>,
    },
    Set {
        kind: PreferenceKindArg,
        value: String,
        #[arg(long)]
        profile: Option<String>,
    },
    Unset {
        kind: PreferenceKindArg,
        #[arg(long)]
        profile: Option<String>,
    },
}
