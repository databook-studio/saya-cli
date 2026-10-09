use clap::{Args, Parser, ValueEnum};

mod command;
mod connection;
mod foundation;
mod investigation;
mod run;
mod value_args;

pub use command::Command;
pub use connection::{ConnectionCommand, ContractsCommand};
pub use foundation::{ConfigCommand, McpCommand, PreferencesCommand, SessionsCommand};
pub use investigation::InvestigationCommand;
pub use run::RunCommand;
pub use value_args::{ClaimKindArg, DoctorTaskArg, ForgetReasonArg, ReviewDecisionArg};

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
    /// Colour palette for the TUI: `dark`, `light`, or `auto`. Defaults to
    /// the `[ui] theme` config setting, then `auto` (honour `COLORFGBG`,
    /// falling back to dark when the terminal reports nothing). Absence of
    /// the flag never overrides config; an explicit `--theme auto` does.
    #[arg(long, value_enum, global = true)]
    pub theme: Option<ThemeArg>,
    /// Show the model's chain-of-thought in the transcript as it streams.
    /// Off by default: thinking is verbose and restates database contents in
    /// prose. Display only — reasoning is never persisted to a session file.
    #[arg(long, global = true)]
    pub show_thinking: bool,
    /// Print extraction-trace diagnostics: why a learned fact was or was not
    /// recorded after a turn (the "memory didn't record" gate). It also
    /// enables local phase timing diagnostics for agent turns.
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

/// The manual Debug impl (F-5): the run command's `--param` values never
/// ride diagnostics.
#[path = "cli_debug.rs"]
mod debug;

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
