use clap::{Args, Parser, ValueEnum};

mod agent;
mod coding;
mod coding_root;
mod command;
mod job;
pub use agent::{FollowUpCommand, StartCommand};
pub use coding::CodingCommand;
pub use coding_root::RootMode;
pub(crate) use job::refusal as job_lane_refusal;
pub(crate) use job::session_spelling as job_session_spelling;
mod connection;
mod foundation;
mod investigation;
mod overrides;
pub(crate) mod print;
pub use print::PrintOptions;
pub(crate) mod retired;
mod run;
mod runtime;
mod value_args;

pub use command::Command;
pub use connection::{ConnectionCommand, ContractsCommand};
pub use foundation::{ConfigCommand, McpCommand, PreferencesCommand, SessionsCommand};
pub use investigation::InvestigationCommand;
pub use retired::RetiredCoding;
pub use run::RunCommand;
pub use runtime::{RuntimeCheckAction, RuntimeCommand};
pub use value_args::{ClaimKindArg, DoctorTaskArg, ForgetReasonArg, ReviewDecisionArg};

const AFTER_HELP: &str = "Examples:\n  \
saya                                   start the interactive REPL\n  \
saya ask \"count orders per region\"     one-shot question\n  \
saya -p \"fix the failing test\"         one-shot headless task\n  \
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
    #[command(flatten)]
    pub print: PrintOptions,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Global options shared by every subcommand and the bare REPL.
#[derive(Debug, Clone, Args, Default)]
pub struct GlobalOptions {
    /// Continue the most recent session (interactive only; a run is not a session).
    #[arg(long = "continue")]
    pub continue_session: bool,
    /// Resume a saved session by id (see `saya config doctor` / /sessions).
    #[arg(long, global = true)]
    pub resume: Option<String>,
    /// Bind the session workspace root to this directory (default: the git worktree top).
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<std::path::PathBuf>,
    /// Seed session grants at launch in the `--allow` grammar (session only).
    #[arg(long, value_name = "SCOPES", value_delimiter = ',')]
    pub allow: Vec<String>,
    /// Deny these program names for the session (repeatable; session only).
    #[arg(long = "deny", value_name = "PROGRAM")]
    pub deny: Vec<String>,
    /// Allow these command prefixes on the host without asking (repeatable).
    #[arg(long = "allow-command", value_name = "PREFIX", global = true)]
    pub allow_command: Vec<String>,
    /// Refuse these command prefixes in every mode (repeatable).
    #[arg(long = "deny-command", value_name = "PREFIX", global = true)]
    pub deny_command: Vec<String>,
    /// Read one turn from this file instead of stdin, then exit (session only).
    #[arg(long = "turn-file", value_name = "PATH")]
    pub turn_file: Option<std::path::PathBuf>,
    /// Start in this permission mode: ask|accept-edits|plan|bypass (session only).
    #[arg(long = "permission-mode", value_name = "MODE")]
    pub permission_mode: Option<String>,
    /// Connection profile to use (overrides `default_profile` in config).
    #[arg(long, global = true)]
    pub profile: Option<String>,
    /// Override the configured AI model for this invocation.
    #[arg(long, global = true)]
    pub model: Option<String>,
    /// Override the configured provider (ollama|openai|openai_compatible|anthropic|gemini).
    #[arg(long, global = true)]
    pub provider: Option<String>,
    /// Limit a job to this many helper children at once.
    #[arg(long, value_name = "N", global = true)]
    pub agents_max_children: Option<usize>,
    /// Limit a job to this many model requests in flight at once.
    #[arg(long, value_name = "N", global = true)]
    pub agents_max_concurrent_model_requests: Option<usize>,
    /// Limit jobs to this many logical model requests.
    #[arg(long, value_name = "N", global = true)]
    pub coding_max_logical_model_requests: Option<usize>,
    /// Limit jobs to this many physical transport attempts.
    #[arg(long, value_name = "N", global = true)]
    pub coding_max_physical_transport_attempts: Option<usize>,
    /// Set the job-wide threshold on provider-reported usage.
    #[arg(long, value_name = "N", global = true)]
    pub coding_max_job_tokens: Option<u64>,
    /// Limit each job to this many elapsed seconds.
    #[arg(long, value_name = "N", global = true)]
    pub coding_max_elapsed_seconds: Option<u64>,
    /// The cost ceiling `-p --max-budget-usd` states. Not a flag of its own:
    /// the one-shot surface folds it into the same `[limits.job]` key the
    /// config layer reads.
    #[arg(skip)]
    pub max_cost_usd: Option<f64>,
    /// Override the configured row cap for query results.
    #[arg(long, value_name = "N", global = true)]
    pub max_rows: Option<usize>,
    /// Answer with the best of N independent attempts (default 1; each is a full agent run).
    #[arg(long, value_name = "N", global = true)]
    pub candidates: Option<usize>,
    /// Additional profiles to query alongside the active one.
    #[arg(long = "include-profile", global = true)]
    pub include_profiles: Vec<String>,
    /// When tool calls need approval: ask|read-only|never|bypass (headless default: never, no `shell`).
    #[arg(long, value_name = "MODE", global = true)]
    pub approval_mode: Option<String>,
    /// Advertise only these model tools (comma-separated, repeatable).
    #[arg(long, value_name = "NAME,...", value_delimiter = ',', global = true)]
    pub tools: Vec<String>,
    /// Output format for subcommands: text | json | ndjson.
    #[arg(
        long,
        value_enum,
        default_value_t = FormatArg::Text,
        global = true,
        alias = "output-format"
    )]
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
    /// Accept security-critical settings from the project's `.saya/config.toml` (off by default).
    #[arg(long, global = true, env = "SAYA_TRUST_PROJECT_CONFIG")]
    pub trust_project_config: bool,
    /// Disable colored output (overrides the detected terminal capability).
    #[arg(long, global = true)]
    pub no_color: bool,
    /// TUI colour palette: dark|light|auto (default: config, then auto).
    #[arg(long, value_enum, global = true)]
    pub theme: Option<ThemeArg>,
    /// Show the model's chain-of-thought in the transcript (off by default).
    #[arg(long, global = true)]
    pub show_thinking: bool,
    /// Print extraction-trace diagnostics and local phase timings.
    #[arg(long, short, global = true)]
    pub verbose: bool,
}

/// The refusal for `--tools` on a surface that does not read it. One text:
/// the CLI dispatch guard and the session's job parser both refuse with
/// it, so the two surfaces cannot disagree about which surfaces honour the
/// flag.
pub(crate) const TOOLS_REFUSAL: &str = "`--tools` narrows the model tools an agent surface \
     advertises, and it is honoured by the interactive session, `saya ask`, `saya run`, \
     and `saya start` only: drop the flag, or state it on one of those";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum FormatArg {
    #[default]
    Text,
    Json,
    #[value(alias = "stream-json")]
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

/// The manual Debug impl: the run command's `--param` values never
/// ride diagnostics.
#[path = "cli_debug.rs"]
mod debug;

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
