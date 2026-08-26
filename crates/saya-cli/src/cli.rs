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
    /// Override the configured AI model for this invocation.
    #[arg(long, global = true)]
    pub model: Option<String>,
    /// Override the configured provider (ollama|openai|openai_compatible|anthropic|gemini).
    #[arg(long, global = true)]
    pub provider: Option<String>,
    /// Override the configured row cap for query results.
    #[arg(long, value_name = "N", global = true)]
    pub max_rows: Option<usize>,
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
    /// Force-disable cloud data sharing for this invocation, overriding any
    /// config layer that enabled it.
    #[arg(long = "no-data-sharing", global = true)]
    pub no_data_sharing: bool,
    /// Accept security-critical settings (`ai.base_url`, `ai.api_key`,
    /// `ai.allow_data_sharing`, `run.read_only`) from the project layer's
    /// `.saya/config.toml`. Off by default: a cloned repository is untrusted.
    #[arg(long, global = true, env = "SAYA_TRUST_PROJECT_CONFIG")]
    pub trust_project_config: bool,
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
        /// Why the directive claim holds — a sentence the model reads alongside
        /// the value so a claim that contradicts a plausible schema reading
        /// (use `return_date`, not `rental_date`) loses less often. Forwarded to
        /// the directive kinds only (grain, time-column, column-role); ignored
        /// for description/alias. Optional: a claim with no reason is the default.
        #[arg(long)]
        reason: Option<String>,
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
    /// claim-id prefix (the `ki-xxxx` `contracts list` abbreviates to), not a
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
