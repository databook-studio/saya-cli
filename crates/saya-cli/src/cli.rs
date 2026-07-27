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
        profile: String,
    },
    Schema {
        profile: String,
        #[arg(long)]
        refresh: bool,
    },
}
